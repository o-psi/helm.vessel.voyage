<?php

namespace App\Http\Middleware;

use App\Services\ConsoleAccess;
use Closure;
use Illuminate\Http\Request;

class ConsoleHeaders
{
    public function handle(Request $request, Closure $next)
    {
        abort_unless(ConsoleAccess::enabled(), 404);
        // Never accept an insecure remote login, even with proxy misconfiguration.
        abort_unless($request->isSecure() || (app()->environment('local', 'testing') && in_array($request->getHost(), ['localhost', '127.0.0.1', '[::1]'], true)), 403);
        $response = $next($request);
        $response->headers->set('Cache-Control', 'no-store, private');
        $response->headers->set('X-Frame-Options', 'DENY');
        $response->headers->set('X-Content-Type-Options', 'nosniff');
        $response->headers->set('Referrer-Policy', 'no-referrer');
        $response->headers->set('Content-Security-Policy', "frame-ancestors 'none'; object-src 'none'; base-uri 'self'");
        return $response;
    }
}
