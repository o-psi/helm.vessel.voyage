<?php
return [
    'enabled' => (bool) env('HELM_WEB_ENABLED', false),
    // Retained only for explicit legacy migration; normal bootstrap never uses Node.
    'legacy_gateway_enabled' => (bool) env('HELM_WEB_LEGACY_GATEWAY_ENABLED', false),
    'gateway_secret' => env('HELM_WEB_GATEWAY_SECRET', ''),
    'gateway_path' => '/console/socket',
    'gateway_admin_url' => env('HELM_WEB_GATEWAY_ADMIN_URL', 'http://127.0.0.1:8787'),
];
