<?php
// Intercept cURL at the PHP namespace boundary to verify production options and
// bounded callbacks without weakening TLS or requiring a public test service.
namespace App\Services {
    function curl_init($url) { $GLOBALS['url']=$url; return new \stdClass; }
    function curl_setopt_array($handle,$options) { $GLOBALS['options']=$options; return true; }
    function curl_exec($handle) {
        $o=$GLOBALS['options']; $body='{"ok":true}';
        if ($GLOBALS['oversize'] ?? false) $body=str_repeat('x',65537);
        return $o[CURLOPT_WRITEFUNCTION]($handle,$body)===strlen($body);
    }
    function curl_getinfo($handle,$key) { return $key===CURLINFO_RESPONSE_CODE ? ($GLOBALS['status'] ?? 200) : 'application/json'; }
    function curl_close($handle) {}
}
namespace {
    require __DIR__.'/../app/Services/PublicVesselHttp.php';
    class Fixture extends App\Services\PublicVesselHttp {
        protected function resolve(string $host): array { return ['2606:4700:4700::1111','8.8.8.8']; }
    }
    $n=0;
    function check($ok,$message) { global $n; if (!$ok) throw new RuntimeException($message); $n++; }
    $http=new Fixture;
    check($http->post('https://vessel.example','/v1/vessel/command',['protocol'=>1])===['ok'=>true],'response');
    $o=$GLOBALS['options'];
    check($GLOBALS['url']==='https://vessel.example/v1/vessel/command','exact URL');
    check($o[CURLOPT_RESOLVE]===['vessel.example:443:[2606:4700:4700::1111]'],'IPv6 connection pin');
    foreach ([CURLOPT_FRESH_CONNECT,CURLOPT_FORBID_REUSE,CURLOPT_SSL_VERIFYPEER] as $key) check($o[$key]===true,'secure option');
    check($o[CURLOPT_SSL_VERIFYHOST]===2,'TLS hostname');
    check($o[CURLOPT_PROXY]==='' && $o[CURLOPT_NOPROXY]==='*','proxy disabled');
    check($o[CURLOPT_FOLLOWLOCATION]===false && $o[CURLOPT_MAXREDIRS]===0,'redirect disabled');
    check($o[CURLOPT_PROTOCOLS]===CURLPROTO_HTTPS,'HTTPS only');
    check($o[CURLOPT_CONNECTTIMEOUT_MS]===3000 && $o[CURLOPT_TIMEOUT_MS]===5000,'deadlines');
    check(($o[CURLOPT_HEADERFUNCTION])(null,str_repeat('a',16385))===0,'header bound');
    $GLOBALS['oversize']=true;
    try { $http->post('https://vessel.example','/v1/vessel/command',[]); throw new LogicException('oversize accepted'); }
    catch (RuntimeException $e) { check(true,'body bound'); }
    $GLOBALS['oversize']=false; $GLOBALS['status']=302;
    try { $http->post('https://vessel.example','/v1/vessel/command',[]); throw new LogicException('redirect accepted'); }
    catch (RuntimeException $e) { check(true,'redirect refused'); }
    echo "PASS $n PHP HTTPS transport checks\n";
}
