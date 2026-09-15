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
        return redirect()->route('home');
    }
    public function ticket(Request $request) {
        $value = $request->validate(['vessel' => ['required', 'uuid']]);
        return response()->json(['ticket' => ConsoleAccess::ticket($request, $value['vessel'])]);
    }
}
