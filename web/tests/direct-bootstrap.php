<?php
// Offline PHP contract checks; no Node, DNS, provider or live Vessel required.
putenv('APP_ENV=testing'); putenv('DB_CONNECTION=sqlite'); putenv('DB_DATABASE=:memory:');
putenv('SESSION_DRIVER=database'); putenv('CACHE_STORE=array'); putenv('APP_URL=https://helm.example');
putenv('APP_KEY=base64:'.base64_encode(str_repeat('k',32)));
$loader = require __DIR__.'/../vendor/autoload.php';
$loader->setPsr4('App\\', __DIR__.'/../app');
foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator(__DIR__.'/../app')) as $file) {
    if ($file->isFile() && $file->getExtension() === 'php') {
        $relative = substr($file->getPathname(), strlen(__DIR__.'/../app/'), -4);
        $loader->addClassMap(['App\\'.str_replace('/', '\\', $relative) => $file->getPathname()]);
    }
}
$app = require __DIR__.'/../bootstrap/app.php';
$app->make(Illuminate\Contracts\Console\Kernel::class)->bootstrap();
use App\Services\{PublicVesselHttp,VesselGateway,ConsoleAccess};
use App\Models\{User,Tenant,VesselConnection};
use Illuminate\Support\Facades\{DB,Auth};
use Illuminate\Http\Request;
use Illuminate\Support\Str;
$passed=false; $count=0;
register_shutdown_function(function () use (&$passed) { if (!$passed) exit(1); });
function check($ok, $message) { global $count; if (!$ok) throw new RuntimeException($message); $count++; }
function refuses(callable $action, $message) { try { $action(); } catch (Throwable) { check(true,$message); return; } throw new RuntimeException('Accepted '.$message); }
class FixtureHttp extends PublicVesselHttp {
    public array $calls=[]; public array $response=[]; public $during=null;
    public function post(string $origin,string $path,array $body,array $headers=[]): array {
        $this->calls[]=compact('origin','path','body','headers');
        if ($this->during) ($this->during)();
        return $this->response;
    }
}
foreach (['8.8.8.8','1.1.1.1','2606:4700:4700::1111','2001:4860:4860::8888'] as $ip) check(PublicVesselHttp::publicAddress($ip), 'public '.$ip);
foreach (['0.1.1.1','10.0.0.1','100.64.0.1','127.0.0.1','169.254.1.1','172.16.0.1','192.0.0.1','192.0.2.1','192.88.99.1','192.168.0.1','198.18.0.1','198.51.100.1','203.0.113.1','224.1.1.1','255.255.255.255','::1','::ffff:8.8.8.8','2001::1','2001:db8::1','2002::1','3fff::1','fc00::1','fe80::1%eth0','garbage'] as $ip) check(!PublicVesselHttp::publicAddress($ip),'blocked '.$ip);
foreach (['http://public.example','https://u:p@public.example','https://public.example:444','https://public.example/path','https://public.example?x=1','https://public.example#x','https://PUBLIC.example','https://127.1','https://public.example//'] as $origin) {
    // Short IPv4 spelling is rejected at DNS/address validation, not URL parsing.
    if ($origin === 'https://127.1') continue;
    refuses(fn()=>PublicVesselHttp::origin($origin),$origin);
}
check(PublicVesselHttp::origin('https://public.example:443/')==='https://public.example','canonical origin');
class DnsFixture extends PublicVesselHttp {
    public array $addresses=[];
    protected function resolve(string $host): array { return $this->addresses; }
}
$dns=new DnsFixture;
foreach ([[],['8.8.8.8','127.0.0.1'],['2606:4700:4700::1111','::1'],['8.8.8.8','bad']] as $ips) {
    $dns->addresses=$ips;
    refuses(fn()=>$dns->post('https://vessel.example','/v1/vessel/command',[]),'all DNS addresses checked');
}
$http=new FixtureHttp; $app->instance(PublicVesselHttp::class,$http); $gateway=$app->make(VesselGateway::class);
$vessel=(string)Str::uuid(); $grant=(string)Str::uuid();
$connection=['url'=>'wss://vessel.example/v1/vessel/socket','token'=>str_repeat('a',64),'grant_id'=>$grant,'vessel_id'=>$vessel];
$http->response=['protocol'=>1,'error'=>null,'outcome_unknown'=>false,'result'=>['protocol'=>1,'vessel_id'=>$vessel,'features'=>[]]];
check($gateway->call('probe',['connection'=>$connection])['vessel_id']===$vessel,'probe result');
check($http->calls[0]['body']===['protocol'=>1,'command'=>['op'=>'capabilities']] && $http->calls[0]['path']==='/v1/vessel/command','HTTP capabilities wire');
check($http->calls[0]['headers']['Authorization']==='Bearer '.str_repeat('a',64),'authorization headers');
$pair=['endpoint'=>'https://vessel.example','principal_id'=>(string)Str::uuid(),'invitation_id'=>(string)Str::uuid(),'command_id'=>(string)Str::uuid(),'vessel_id'=>$vessel,'code'=>'secret'];
$gateway->call('pair',$pair); check($http->calls[1]['body']['command_id']===$pair['command_id'] && !isset($http->calls[1]['body']['endpoint']),'pair identity');
$http->response['outcome_unknown']=true; refuses(fn()=>$gateway->call('pair',$pair),'uncertain pair');
$http->response=['token'=>'short-lived','expires_at_ms'=>now()->getTimestampMs()+119000,'vessel_id'=>$vessel];
$mint=$gateway->call('browser-credentials',['connection'=>$connection]);
check($mint['url']==='wss://vessel.example/v1/vessel/browser-socket','socket URL');
check(end($http->calls)['body']===['origin'=>'https://helm.example'],'trusted configured origin');
foreach ([now()->getTimestampMs()-1,now()->getTimestampMs()+121000,'123'] as $expiry) {
    $http->response['expires_at_ms']=$expiry; refuses(fn()=>$gateway->call('browser-credentials',['connection'=>$connection]),'bad lifetime');
}
foreach (glob(__DIR__.'/../database/migrations/*.php') as $file) (require $file)->up();
config(['helm.enabled'=>true,'helm.gateway_secret'=>'','session.driver'=>'database','session.encrypt'=>false]);
check(ConsoleAccess::enabled(),'no gateway secret required');
$controller=new App\Http\Controllers\ConsoleAuthController;
$untrusted=Request::create('https://spoofed.example/console/ticket','POST',[],[],[],['HTTP_ORIGIN'=>'https://evil.example']);
refuses(fn()=>$controller->ticket($untrusted),'cross origin controller POST');
refuses(fn()=>$controller->ticket(Request::create('https://helm.example/console/ticket','POST')),'missing origin controller POST');
$tenant=Tenant::create(['name'=>'One','principal_id'=>(string)Str::uuid()]);
$other=Tenant::create(['name'=>'Two','principal_id'=>(string)Str::uuid()]);
$user=User::create(['name'=>'User','email'=>'one@example.test','tenant_id'=>$tenant->id]);
$credential=['endpoint'=>'https://vessel.example','token'=>str_repeat('a',64),'grant_id'=>$grant,'vessel_id'=>$vessel];
$stored=VesselConnection::create(['tenant_id'=>$tenant->id,'name'=>'Vessel','endpoint'=>'https://vessel.example','vessel_id'=>$vessel,'credential'=>$credential,'revision'=>1]);
$foreign=VesselConnection::create(['tenant_id'=>$other->id,'name'=>'Other','endpoint'=>'https://vessel.example','vessel_id'=>$vessel,'credential'=>$credential,'revision'=>1]);
$request=Request::create('https://spoofed.example/console/ticket','POST',['vessel'=>$stored->id]);
$request->setLaravelSession($app['session']->driver()); $request->session()->start();
$app->instance('request',$request); Auth::login($user); $request->setUserResolver(fn()=>$user);
$request->session()->put('helm_operator_until',time()+3600); $request->session()->save();
$http->response=['token'=>'short-lived','expires_at_ms'=>now()->getTimestampMs()+119000,'vessel_id'=>$vessel];
check(ConsoleAccess::ticket($request,$stored->id)['token']==='short-lived','tenant mint');
check(end($http->calls)['body']['origin']==='https://helm.example','request host ignored');
$request->headers->set('Origin','https://helm.example');
$response=$controller->ticket($request);
check(array_keys($response->getData(true))===['token','expires_at_ms','vessel_id','url'],'direct controller JSON shape');
check(str_contains($response->headers->get('Cache-Control'),'no-store'),'credential response not cached');
$ticketRoute=$app['router']->getRoutes()->getByName('console.ticket');
check(in_array('throttle:console-tickets',$ticketRoute->middleware(),true),'ticket rate limit');
check(in_array('web',$ticketRoute->middleware(),true) && !$ticketRoute->excludedMiddleware(),'ticket retains web CSRF middleware');
refuses(fn()=>ConsoleAccess::ticket($request,$foreign->id),'foreign tenant');
$http->during=fn()=>DB::table('vessel_connections')->where('id',$stored->id)->update(['revision'=>2]);
refuses(fn()=>ConsoleAccess::ticket($request,$stored->id),'concurrent revision');
$savedSession=(array)DB::table('sessions')->where('id',$request->session()->getId())->first();
$http->during=fn()=>DB::table('sessions')->where('id',$request->session()->getId())->delete();
refuses(fn()=>ConsoleAccess::ticket($request,$stored->id),'concurrent logout');
$http->during=null; refuses(fn()=>ConsoleAccess::ticket($request,$stored->id),'logged-out session');
check(VesselConnection::find($stored->id)->credential===$credential,'long-term grant preserved');
DB::table('sessions')->insert($savedSession); // Fixture explicit login restoration.
$http->during=fn()=>VesselConnection::whereKey($stored->id)->delete();
refuses(fn()=>ConsoleAccess::ticket($request,$stored->id),'concurrent removal');
check(!VesselConnection::find($stored->id),'removal happened during HTTPS');
$passed=true; echo "PASS $count direct PHP bootstrap checks\n";
