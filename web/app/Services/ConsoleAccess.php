<?php
namespace App\Services;
use App\Models\User;
use App\Models\VesselConnection;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\DB;
final class ConsoleAccess {
    public static function enabled(): bool {
        return config('helm.enabled') && config('session.driver') === 'database' && !config('session.encrypt');
    }
    public static function authenticated(Request $request): bool {
        return self::enabled() && $request->user()?->tenant_id
            && $request->session()->get('helm_operator_until', 0) > time();
    }
    private static function current(Request $request, VesselConnection $connection): bool {
        $user = User::find($request->user()->id);
        $session = DB::table('sessions')->where('id', $request->session()->getId())->first();
        if (!$user || $user->tenant_id !== $connection->tenant_id || !$session
            || (int) $session->user_id !== (int) $user->id
            || $session->last_activity <= time() - config('session.lifetime') * 60) return false;
        $payload = base64_decode($session->payload, true);
        $data = config('session.serialization') === 'json' ? json_decode($payload ?: '', true) : @unserialize($payload ?: '', ['allowed_classes'=>false]);
        $fresh = VesselConnection::where('tenant_id', $connection->tenant_id)->find($connection->id);
        return is_array($data) && ($data['helm_operator_until'] ?? 0) > time()
            && (int) ($data[auth()->getName()] ?? 0) === (int) $user->id
            && $fresh && $fresh->revision === $connection->revision && $fresh->endpoint === $connection->endpoint
            && $fresh->credential === $connection->credential;
    }
    public static function ticket(Request $request, string $vessel): array {
        abort_unless(self::authenticated($request), 401);
        $connection = VesselConnection::where('tenant_id', $request->user()->tenant_id)->findOrFail($vessel);
        // Never save the in-flight session here: logout may already have deleted it.
        abort_unless(self::current($request, $connection), 401);
        $credential = $connection->credential;
        try {
            $result = app(VesselGateway::class)->call('browser-credentials', ['connection'=>[
                'url'=>preg_replace('/^https:/', 'wss:', VesselGateway::endpoint($connection->endpoint)).'/v1/vessel/socket',
                'token'=>$credential['token'], 'grant_id'=>$credential['grant_id'], 'vessel_id'=>$connection->vessel_id]]);
        } catch (\Throwable) { abort(502, 'Vessel could not issue a browser credential.'); }
        // A logout/removal/update during DNS or HTTPS must not release its result.
        // After this check there is necessarily a delivery race: the Vessel's hard
        // 120-second credential lifetime bounds it; no long-term grant is revoked.
        abort_unless(self::current($request, $connection), 401);
        return $result;
    }
}
