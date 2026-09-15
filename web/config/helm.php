<?php

return [
    // Disabled until explicitly provisioned. Password is a password_hash() result.
    'enabled' => (bool) env('HELM_WEB_ENABLED', false),
    'password_hash' => env('HELM_WEB_PASSWORD_HASH', ''),
    'gateway_secret' => env('HELM_WEB_GATEWAY_SECRET', ''),
    'gateway_path' => '/console/socket',
    // Aliases only: credentials and upstream addresses belong to the gateway.
    'vessels' => array_values(array_filter(array_map('trim', explode(',', env('HELM_WEB_VESSELS', ''))))),
];
