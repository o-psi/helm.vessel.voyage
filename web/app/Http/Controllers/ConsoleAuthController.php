<?php
namespace App\Http\Controllers;
use App\Services\ConsoleAccess;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\Auth;
use Illuminate\Support\Facades\DB;
class ConsoleAuthController extends Controller {
    public function logout(Request $request) {
        DB::table('web_gateway_tickets')->where('session_id', $request->session()->getId())->delete();
        Auth::logout();
        $request->session()->invalidate();
        $request->session()->regenerateToken();
        return redirect()->route('home')->with('status', 'Signed out. Existing direct Vessel credentials expire within 120 seconds; socket closure may take up to 3 additional seconds. Already admitted work is not cancelled. Stored Vessel grants have not been revoked.');
    }
    public function ticket(Request $request) {
        // Browser POST only: trust configured origin, never Host/forwarded Host.
        abort_unless($request->header('Origin') === \App\Services\VesselGateway::endpoint(config('app.url')), 403);
        $value = $request->validate(['vessel' => ['required', 'uuid']]);
        return response()->json(ConsoleAccess::ticket($request, $value['vessel']))->header('Cache-Control', 'no-store, private');
    }
}
