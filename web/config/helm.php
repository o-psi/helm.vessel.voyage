<?php
return [
    'enabled' => (bool) env('HELM_WEB_ENABLED', false),
    'gateway_secret' => env('HELM_WEB_GATEWAY_SECRET', ''),
    'gateway_path' => '/console/socket',
    'gateway_admin_url' => env('HELM_WEB_GATEWAY_ADMIN_URL', 'http://127.0.0.1:8787'),
];
