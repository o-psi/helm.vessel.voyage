<?php
namespace App\Http\Controllers;
use App\Models\VesselConnection;
use App\Models\VesselPairing;
use App\Services\VesselGateway;
use Illuminate\Http\Request;
use Illuminate\Support\Str;
class VesselConnectionController extends Controller {
    public function index(Request $request) {
        return view('connections.index', ['tenant' => $request->user()->tenant,
            'connections' => VesselConnection::where('tenant_id', $request->user()->tenant_id)->get(),
            'pairings' => VesselPairing::where('tenant_id', $request->user()->tenant_id)->where('status', 'pending')->get()]);
    }
    public function store(Request $request, VesselGateway $gateway) {
        $data = $request->validate(['name' => ['required','string','max:100'], 'credential' => ['required','string','max:16384']]);
        try {
            $credential = json_decode($data['credential'], true, flags: JSON_THROW_ON_ERROR);
            $this->save($request, $gateway, $data['name'], $credential);
        } catch (\Throwable) { return back()->withErrors(['connection' => 'Unable to add this Vessel. Check the public endpoint, identity and grant.']); }
        return redirect()->route('connections')->with('status', 'Vessel connected.');
    }
    private function save(Request $request, VesselGateway $gateway, string $name, array $credential): void {
        $endpoint = VesselGateway::endpoint($credential['endpoint'] ?? '');
        foreach (['grant_id','vessel_id'] as $key) abort_unless(Str::isUuid($credential[$key] ?? ''), 422);
        abort_unless(is_string($credential['token'] ?? null) && preg_match('/^[a-f0-9]{64}$/Di', $credential['token']), 422);
        $probe = $gateway->call('probe', ['connection' => ['url' => preg_replace('/^https:/','wss:',$endpoint).'/v1/vessel/socket',
            'token' => $credential['token'], 'grant_id' => $credential['grant_id'], 'vessel_id' => $credential['vessel_id']]]);
        abort_unless(($probe['vessel_id'] ?? null) === $credential['vessel_id'], 422);
        $existing = VesselConnection::where('tenant_id',$request->user()->tenant_id)->where('vessel_id',$credential['vessel_id'])->first();
        if ($existing) {
            $existing->update(['name'=>$name,'endpoint'=>$endpoint,'credential'=>$credential,'revision'=>$existing->revision+1]);
        } else {
            abort_if(VesselConnection::where('tenant_id',$request->user()->tenant_id)->count() >= 64, 422);
            VesselConnection::create(['tenant_id'=>$request->user()->tenant_id,'name'=>$name,'endpoint'=>$endpoint,'vessel_id'=>$credential['vessel_id'],'credential'=>$credential]);
        }
    }
    public function pair(Request $request, VesselGateway $gateway) {
        $data = $request->validate(['name'=>['required','string','max:100'], 'invitation'=>['required','string','max:16384']]);
        try {
            $invitation = json_decode($data['invitation'],true,flags:JSON_THROW_ON_ERROR);
            abort_unless(($invitation['principal_id'] ?? null) === $request->user()->tenant->principal_id,422);
            foreach (['invitation_id','vessel_id'] as $key) abort_unless(Str::isUuid($invitation[$key] ?? ''),422);
            abort_unless(is_string($invitation['code'] ?? null) && strlen($invitation['code']) <= 1024,422);
            abort_unless(($invitation['expires_at_ms'] ?? 0) > now()->getTimestampMs(),422);
            $payload = ['endpoint'=>VesselGateway::endpoint($invitation['endpoint'] ?? ''), 'principal_id'=>$invitation['principal_id'],
                'invitation_id'=>$invitation['invitation_id'], 'vessel_id'=>$invitation['vessel_id'], 'code'=>$invitation['code'], 'command_id'=>(string) Str::uuid()];
            abort_if(VesselPairing::where('tenant_id',$request->user()->tenant_id)->where('status','pending')->count() >= 16,422);
            $pairing = VesselPairing::create(['tenant_id'=>$request->user()->tenant_id,'name'=>$data['name'],'request'=>$payload]);
            return $this->completePair($request,$gateway,$pairing);
        } catch (\Throwable) { return back()->withErrors(['connection'=>'Pairing not confirmed. Any saved attempt remains available below; retry that same attempt rather than submit a new invitation.']); }
    }
    public function retry(Request $request, VesselGateway $gateway, string $id) {
        $pairing = VesselPairing::where('tenant_id',$request->user()->tenant_id)->findOrFail($id);
        abort_unless($pairing->status === 'pending',409);
        try { return $this->completePair($request,$gateway,$pairing); }
        catch (\Throwable) { return back()->withErrors(['connection'=>'Pairing is still unconfirmed. Original command identity retained.']); }
    }
    private function completePair(Request $request,VesselGateway $gateway,VesselPairing $pairing) {
        $response = $gateway->call('pair',$pairing->request);
        $credential = $response['result'] ?? $response;
        abort_unless(($credential['principal_id'] ?? null) === $request->user()->tenant->principal_id
            && ($credential['vessel_id'] ?? null) === $pairing->request['vessel_id']
            && ($credential['endpoint'] ?? null) === $pairing->request['endpoint'],422);
        $this->save($request,$gateway,$pairing->name,$credential);
        $pairing->delete(); // Consumed invitation secret no longer needed.
        return redirect()->route('connections')->with('status','Vessel paired.');
    }
    public function destroy(Request $request,string $id) {
        $connection = VesselConnection::where('tenant_id',$request->user()->tenant_id)->findOrFail($id);
        $connection->delete();
        return back()->with('status','Connection removed. Open sockets expire within 60 seconds. Revoke its grant on the Vessel if it is no longer needed.');
    }
}
