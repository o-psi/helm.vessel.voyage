<?php
namespace App\Services;
use App\Models\VesselConnection;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\DB;
final class ConsoleAccess {
    public static function enabled(): bool {
        return config('helm.enabled') && config('session.driver') === 'database' && !config('session.encrypt') && strlen(config('helm.gateway_secret', '')) >= 32;
    }
    public static function authenticated(Request $request): bool {
        return self::enabled() && $request->user()?->tenant_id
            && $request->session()->get('helm_operator_until', 0) > time();
    }
    public static function ticket(Request $request, string $vessel): string {
        abort_unless(self::authenticated($request), 401);
        $connection = VesselConnection::where('tenant_id', $request->user()->tenant_id)->findOrFail($vessel);
        // The DB session must be committed before the gateway can redeem the ticket.
        $request->session()->save();
        $token = rtrim(strtr(base64_encode(random_bytes(32)), '+/', '-_'), '=');
        DB::table('web_gateway_tickets')->where('expires_at', '<=', time())->delete();
        DB::table('web_gateway_tickets')->insert([
            'id' => hash('sha256', $token), 'tenant_id' => $connection->tenant_id,
            'user_id' => $request->user()->id, 'connection_id' => $connection->id,
            'connection_revision' => $connection->revision,
            'session_id' => $request->session()->getId(),
            'subject' => hash_hmac('sha256', $request->session()->getId(), config('helm.gateway_secret')),
            'expires_at' => min(time() + 60, $request->session()->get('helm_operator_until')),
        ]);
        return $token;
    }
}
