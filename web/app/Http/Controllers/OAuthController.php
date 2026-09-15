<?php

namespace App\Http\Controllers;

use App\Services\OAuthAccounts;
use App\Services\OAuthProviders;
use Illuminate\Http\RedirectResponse;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\Auth;
use Illuminate\Support\Str;
use RuntimeException;
use Throwable;

class OAuthController extends Controller
{
    public static function providers(): array
    {
        return app(OAuthProviders::class)->availableProviders();
    }

    public function redirect(Request $request, string $provider, OAuthProviders $providers): RedirectResponse
    {
        abort_unless(array_key_exists($provider, $providers->availableProviders()), 404);
        $this->clearAttempt($request);
        try {
            $response = $providers->driver($provider)->redirect();
            $request->session()->put('helm_oauth_attempt', ['provider' => $provider, 'expires' => time() + 600]);
            return $response;
        } catch (Throwable) {
            return $this->failed($request);
        }
    }

    public function callback(Request $request, string $provider, OAuthProviders $providers, OAuthAccounts $accounts): RedirectResponse
    {
        try {
            $attempt = $request->session()->pull('helm_oauth_attempt');
            if (! array_key_exists($provider, $providers->availableProviders())
                || ! is_array($attempt) || ($attempt['provider'] ?? null) !== $provider
                || ($attempt['expires'] ?? 0) < time() || $request->has('error')
                || ! is_string($request->query('code')) || $request->query('code') === '') {
                throw new RuntimeException('Invalid OAuth attempt.');
            }
            // Socialite validates and consumes state before any token exchange.
            $user = $accounts->resolve($provider, $providers->driver($provider)->user());
            $this->clearAttempt($request);
            Auth::login($user);
            $request->session()->regenerate();
            $request->session()->put('helm_operator_until', time() + 8 * 60 * 60);
            $request->session()->put('helm_auth_id', (string) Str::uuid());
            return redirect('/');
        } catch (Throwable) {
            // Deliberately do not report exceptions: provider HTTP bodies can
            // contain tokens or profile data. Never flash callback input either.
            return $this->failed($request);
        }
    }

    private function clearAttempt(Request $request): void
    {
        $request->session()->forget(['helm_oauth_attempt', 'state', 'code_verifier']);
    }

    private function failed(Request $request): RedirectResponse
    {
        $this->clearAttempt($request);
        return redirect('/console/login')->withErrors(['oauth' => 'Sign-in could not be completed. Please try again.']);
    }
}
