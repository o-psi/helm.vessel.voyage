<?php

namespace App\Http\Middleware;

use App\Services\ConsoleAccess;
use Closure;
use Illuminate\Http\Request;

class ConsoleOperator
{
    public function handle(Request $request, Closure $next)
    {
        if (!ConsoleAccess::authenticated($request)) {
            return $request->expectsJson() ? response()->json(['message' => 'Sign in again.'], 401) : redirect()->route('console.login');
        }
        return $next($request);
    }
}
