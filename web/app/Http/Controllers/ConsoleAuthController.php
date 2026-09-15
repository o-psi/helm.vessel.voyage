<?php

namespace App\Http\Controllers;

use App\Services\ConsoleAccess;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\RateLimiter;

class ConsoleAuthController extends Controller
{
    public function login(Request $request)
    {
        // Explicit validation avoids flashing the submitted password on failure.
        $key = 'helm-login:'.hash('sha256', $request->ip());
        abort_if(RateLimiter::tooManyAttempts($key, 5), 429);
        $password = $request->input('password');
        RateLimiter::hit($key, 300);
        if (!is_string($password) || strlen($password) > 1024 || !password_verify($password, config('helm.password_hash'))) {
            return back()->withErrors(['login' => 'Unable to sign in.']);
        }
        RateLimiter::clear($key);
        $request->session()->regenerate();
        $request->session()->put('helm_operator_until', time() + 8 * 3600);
        return redirect()->route('console');
    }

    public function logout(Request $request)
    {
        $request->session()->invalidate();
        $request->session()->regenerateToken();
        return redirect()->route('home');
    }

    public function ticket(Request $request)
    {
        $value = $request->validate(['vessel' => ['required', 'string', 'max:64']]);
        return response()->json(['ticket' => ConsoleAccess::ticket($request, $value['vessel'])]);
    }
}
