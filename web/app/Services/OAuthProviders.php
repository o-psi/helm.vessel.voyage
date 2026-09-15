<?php

namespace App\Services;

use GuzzleHttp\Client;
use Laravel\Socialite\Facades\Socialite;
use Laravel\Socialite\Two\AbstractProvider;

class OAuthProviders
{
    public function availableProviders(): array
    {
        $available = [];
        foreach (config('oauth.providers', []) as $id => $provider) {
            $credentials = config('services.'.$id, []);
            if (is_string($credentials['client_id'] ?? null) && trim($credentials['client_id']) !== ''
                && is_string($credentials['client_secret'] ?? null) && trim($credentials['client_secret']) !== '') {
                $available[$id] = $provider['label'];
            }
        }
        return $available;
    }

    public function driver(string $provider): AbstractProvider
    {
        abort_unless(array_key_exists($provider, $this->availableProviders()), 404);
        // A fresh driver avoids cached user/request state in long-lived workers.
        Socialite::forgetDrivers();
        $driver = Socialite::driver($provider);
        $driver->redirectUrl(rtrim(config('app.url'), '/').'/console/auth/'.$provider.'/callback');
        $driver->setScopes(config('oauth.providers.'.$provider.'.scopes'));
        $driver->enablePKCE();
        $driver->setHttpClient(new Client(['connect_timeout' => 5, 'timeout' => 15]));
        return $driver;
    }
}
