<?php
namespace App\Http\Controllers;
use App\Models\User;
use App\Models\VesselConnection;
use App\Services\ConsoleAccess;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\DB;
class GatewayController extends Controller {
    public function authorizeTicket(Request $request) {
        abort_unless(config('helm.legacy_gateway_enabled') && strlen(config('helm.gateway_secret', '')) >= 32 && ConsoleAccess::enabled() && in_array($request->server('REMOTE_ADDR'), ['127.0.0.1', '::1'], true)
            && is_string($request->bearerToken()) && hash_equals(config('helm.gateway_secret'), $request->bearerToken()), 403);
        $token = $request->input('ticket');
        abort_unless(is_string($token) && preg_match('/^[A-Za-z0-9_-]{43}$/D', $token), 403);
        $result = DB::transaction(function () use ($token) {
            $id = hash('sha256', $token);
            $ticket = DB::table('web_gateway_tickets')->where('id', $id)->first();
            abort_unless($ticket && $ticket->expires_at > time(), 403);
            abort_unless(DB::table('web_gateway_tickets')->where('id', $id)->delete() === 1, 403);
            $user = User::find($ticket->user_id);
            abort_unless($user && $user->tenant_id === $ticket->tenant_id, 403);
            // Database-backed sessions are mandatory for revocation and tenant binding.
            $session = DB::table('sessions')->where('id', $ticket->session_id)->first();
            abort_unless($session && (int) $session->user_id === (int) $ticket->user_id
                && $session->last_activity > time() - config('session.lifetime') * 60, 403);
            $payload = base64_decode($session->payload, true);
            $data = config('session.serialization') === 'json' ? json_decode($payload ?: '', true) : @unserialize($payload ?: '', ['allowed_classes' => false]);
            abort_unless(is_array($data) && ($data['helm_operator_until'] ?? 0) > time()
                && (int) ($data[auth()->getName()] ?? 0) === (int) $ticket->user_id, 403);
            $connection = VesselConnection::where('tenant_id', $ticket->tenant_id)->find($ticket->connection_id);
            abort_unless($connection && $connection->revision === (int) $ticket->connection_revision, 403);
            $credential = $connection->credential;
            return ['sub' => $ticket->subject, 'tenant' => $ticket->tenant_id, 'vessel' => $connection->id,
                'exp' => $ticket->expires_at, 'connection' => ['url' => preg_replace('/^https:/', 'wss:', $connection->endpoint).'/v1/vessel/socket',
                    'token' => $credential['token'], 'grant_id' => $credential['grant_id'], 'vessel_id' => $connection->vessel_id]];
        });
        return response()->json($result)->header('Cache-Control', 'no-store');
    }
}
