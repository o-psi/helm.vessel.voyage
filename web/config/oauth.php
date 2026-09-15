<?php

return [
    // Extending this registry requires an audited Socialite OAuth2 driver.
    'providers' => [
        'google' => ['label' => 'Google', 'scopes' => ['openid', 'profile', 'email']],
        'x' => ['label' => 'X', 'scopes' => ['users.read', 'tweet.read']],
        'github' => ['label' => 'GitHub', 'scopes' => ['read:user', 'user:email']],
    ],
];
