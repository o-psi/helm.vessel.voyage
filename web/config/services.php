<?php

// Callback URLs are derived from the trusted deployment URL, never the request Host.
$oauth = static fn (string $provider, string $prefix): array => [
    'client_id' => env($prefix.'_CLIENT_ID', ''),
    'client_secret' => env($prefix.'_CLIENT_SECRET', ''),
    'redirect' => rtrim(env('APP_URL', 'http://localhost'), '/').'/console/auth/'.$provider.'/callback',
];

return [
    'google' => $oauth('google', 'GOOGLE'),
    'x' => $oauth('x', 'X'),
    'github' => $oauth('github', 'GITHUB'),

    /*
    |--------------------------------------------------------------------------
    | Third Party Services
    |--------------------------------------------------------------------------
    |
    | This file is for storing the credentials for third party services such
    | as Resend, Postmark, AWS, and more. This file provides the de facto
    | location for this type of information, allowing packages to have
    | a conventional file to locate the various service credentials.
    |
    */

    'postmark' => [
        'key' => env('POSTMARK_API_KEY'),
    ],

    'resend' => [
        'key' => env('RESEND_API_KEY'),
    ],

    'ses' => [
        'key' => env('AWS_ACCESS_KEY_ID'),
        'secret' => env('AWS_SECRET_ACCESS_KEY'),
        'region' => env('AWS_DEFAULT_REGION', 'us-east-1'),
    ],

    'slack' => [
        'notifications' => [
            'bot_user_oauth_token' => env('SLACK_BOT_USER_OAUTH_TOKEN'),
            'channel' => env('SLACK_BOT_USER_DEFAULT_CHANNEL'),
        ],
    ],

];
