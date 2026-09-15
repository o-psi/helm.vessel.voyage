<?php

namespace App\Services;

use Illuminate\Http\Request;
use Illuminate\Support\Str;

final class ConsoleAccess
{
    public static function enabled(): bool
    {
        return config('helm.enabled') && strlen(config('helm.gateway_secret', '')) >= 32
            && (password_get_info(config('helm.password_hash', ''))['algo'] ?? null) !== null
            && count(config('helm.vessels', [])) > 0;
    }

    public static function authenticated(Request $request): bool
    {
        return self::enabled() && $request->session()->get('helm_operator_until', 0) > time();
    }

    public static function ticket(Request $request, string $vessel): string
    {
        abort_unless(self::authenticated($request), 401);
        abort_unless(in_array($vessel, config('helm.vessels'), true), 422);
        $claims = ['aud' => 'helm-web-gateway', 'sub' => hash_hmac('sha256', $request->session()->getId(), config('helm.gateway_secret')),
            'jti' => (string) Str::uuid(), 'exp' => min(time() + 60, $request->session()->get('helm_operator_until')), 'vessel' => $vessel];
        $encode = fn (string $bytes) => rtrim(strtr(base64_encode($bytes), '+/', '-_'), '=');
        $payload = $encode(json_encode($claims, JSON_THROW_ON_ERROR));
        return $payload.'.'.$encode(hash_hmac('sha256', $payload, config('helm.gateway_secret'), true));
    }
}
