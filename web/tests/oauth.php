<?php

// Offline, isolated in-memory SQLite checks. No provider credentials/network.
// Run from web/: php -d extension=iconv tests/oauth.php
// Real Socialite callbacks are mocked at the Guzzle transport, not at user().
putenv('APP_ENV=testing');
putenv('DB_CONNECTION=sqlite');
putenv('DB_DATABASE=:memory:');
putenv('SESSION_DRIVER=array');
putenv('CACHE_STORE=array');
putenv('APP_KEY=base64:'.base64_encode(str_repeat('k', 32)));
require __DIR__.'/../vendor/autoload.php';
$app = require __DIR__.'/../bootstrap/app.php';
$app->make(Illuminate\Contracts\Console\Kernel::class)->bootstrap();

use App\Http\Controllers\OAuthController;
use App\Models\OAuthIdentity;
use App\Models\Tenant;
use App\Models\User;
use App\Services\OAuthAccounts;
use App\Services\OAuthProviders;
use GuzzleHttp\Client;
use GuzzleHttp\Handler\MockHandler;
use GuzzleHttp\HandlerStack;
use GuzzleHttp\Middleware;
use GuzzleHttp\Psr7\Response;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\Auth;
use Illuminate\Support\Facades\DB;
use Illuminate\Support\Str;
use Laravel\Socialite\Two\AbstractProvider;

$count = 0;
$passed = false;
// Laravel's exception renderer can exit zero for a standalone script.
register_shutdown_function(function () use (&$passed) { if (! $passed) { exit(1); } });
function check(bool $condition, string $message): void {
    global $count;
    if (! $condition) { throw new RuntimeException($message); }
    $count++;
}
function requestFor(string $path, array $query = []): Request {
    global $app;
    $request = Request::create('https://helm.example'.$path, 'GET', $query);
    $request->setLaravelSession($app['session']->driver());
    $app->instance('request', $request);
    return $request;
}
class MockProviders extends OAuthProviders {
    public array $history = [];
    public array $responses = [];
    public function driver(string $provider): AbstractProvider {
        $driver = parent::driver($provider);
        $handler = HandlerStack::create(new MockHandler($this->responses));
        $handler->push(Middleware::history($this->history));
        return $driver->setHttpClient(new Client(['handler' => $handler]));
    }
}

// Upgrade an existing initial-schema database, retaining password user data.
foreach (glob(__DIR__.'/../database/migrations/0001_*.php') as $path) { (require $path)->up(); }
$legacy = User::create(['name' => 'Legacy', 'email' => 'same@example.test', 'password' => 'legacy-password']);
$oldHash = $legacy->password;
$migration = require __DIR__.'/../database/migrations/2026_09_01_000000_create_oauth_tenants.php';
$migration->up();
$legacy->refresh();
check($legacy->tenant_id === null && $legacy->password === $oldHash, 'legacy preserved without tenant');

foreach (['google', 'x', 'github'] as $id) { config(['services.'.$id.'.client_id' => '', 'services.'.$id.'.client_secret' => '']); }
check(OAuthController::providers() === [], 'no incomplete providers');
config(['services.google.client_id' => 'fixture']);
check(OAuthController::providers() === [], 'client id alone not sufficient');
foreach (['google', 'x', 'github'] as $id) {
    config(['services.'.$id.'.client_id' => 'fixture-id', 'services.'.$id.'.client_secret' => 'fixture-secret']);
}
config(['app.url' => 'https://helm.example/']);
check(OAuthController::providers() === ['google' => 'Google', 'x' => 'X', 'github' => 'GitHub'], 'registry contract');
$controller = new OAuthController;
$accounts = new OAuthAccounts;
$providers = new MockProviders;
$app->instance(OAuthProviders::class, $providers);
$session = $app['session']->driver();
$session->start();
$profile = (new Laravel\Socialite\Two\User)->map(['id' => 'g-1', 'name' => 'Alice', 'email' => 'same@example.test']);
$google = $accounts->resolve('google', $profile);
$github = $accounts->resolve('github', $profile);
check($google->id !== $github->id && $google->id !== $legacy->id, 'never link by email');
check($google->tenant_id !== $github->tenant_id, 'distinct personal tenants');
check(Str::isUuid($google->tenant_id) && Str::isUuid($google->tenant->principal_id), 'UUID schema');
check($google->tenant->principal_id !== $github->tenant->principal_id, 'distinct pairing principals');
check($google->password === null && $google->email_verified_at === null, 'no password or trusted email assertion');
check($accounts->resolve('google', $profile)->id === $google->id && Tenant::count() === 2, 'repeat identity no duplicate tenant');
// Inject a failure after user and tenant inserts, proving transaction rollback.
OAuthIdentity::creating(function ($identity) { if ($identity->subject === 'fail') { throw new RuntimeException('fixture failure'); } });
try { $accounts->resolve('google', (new Laravel\Socialite\Two\User)->map(['id' => 'fail'])); } catch (RuntimeException) {}
check(Tenant::count() === 2 && User::count() === 3, 'failed identity insert leaves no orphan');
OAuthIdentity::flushEventListeners();
try { DB::table('oauth_identities')->insert(['provider' => 'google', 'subject' => 'g-1', 'user_id' => $github->id]); throw new RuntimeException('duplicate accepted'); }
catch (Illuminate\Database\UniqueConstraintViolationException) { check(true, 'unique provider subject'); }

foreach (['google', 'x', 'github'] as $provider) {
    $providers->history = [];
    $request = requestFor('/auth/'.$provider);
    $response = $controller->redirect($request, $provider, $providers);
    parse_str(parse_url($response->getTargetUrl(), PHP_URL_QUERY), $query);
    check(($query['redirect_uri'] ?? '') === 'https://helm.example/auth/'.$provider.'/callback', $provider.' exact trusted callback');
    check(strlen($query['state'] ?? '') >= 32, $provider.' random state');
    check(($query['code_challenge_method'] ?? '') === 'S256', $provider.' S256 PKCE');
    $verifier = $session->get('code_verifier');
    check($query['code_challenge'] === rtrim(strtr(base64_encode(hash('sha256', $verifier, true)), '+/', '-_'), '='), $provider.' challenge binds verifier');
    check(! str_contains($query['scope'], 'offline.access'), 'no refresh-token scope');
    $providers->responses = [new Response(200, [], json_encode(['access_token' => 'synthetic-secret-token', 'token_type' => 'Bearer']))];
    $data = match ($provider) {
        'google' => ['sub' => 'callback-google', 'name' => 'Alice', 'email' => 'same@example.test', 'picture' => null],
        'x' => ['data' => ['id' => 'callback-x', 'username' => 'alice', 'name' => 'Alice', 'profile_image_url' => null]],
        'github' => ['id' => 345, 'node_id' => 'fixture-node', 'login' => 'alice', 'name' => 'Alice', 'email' => 'same@example.test', 'avatar_url' => null],
    };
    $providers->responses[] = new Response(200, [], json_encode($data));
    if ($provider === 'github') {
        $providers->responses[] = new Response(200, [], '[{"email":"same@example.test","primary":true,"verified":true}]');
    }
    $before = $session->getId();
    $request = requestFor('/auth/'.$provider.'/callback', ['code' => 'synthetic-code', 'state' => $query['state']]);
    $response = $controller->callback($request, $provider, $providers, $accounts);
    check($response->getTargetUrl() === 'https://helm.example' || $response->getTargetUrl() === 'http://localhost' || str_ends_with($response->getTargetUrl(), '/'), $provider.' callback redirects root: '.$response->getTargetUrl());
    check(Auth::check() && Auth::user()->tenant_id !== null, $provider.' authenticated tenant');
    check($session->getId() !== $before && Str::isUuid($session->get('helm_auth_id')), $provider.' regenerated session and ticket identity');
    check(abs($session->get('helm_operator_until') - (time() + 28800)) <= 1, $provider.' eight hour expiry');
    check(! $session->has('state') && ! $session->has('code_verifier') && ! $session->has('helm_oauth_attempt'), 'attempt consumed');
    parse_str((string) $providers->history[0]['request']->getBody(), $exchange);
    check(($exchange['code_verifier'] ?? '') === $verifier, $provider.' verifier sent in token exchange');
    check(! str_contains(json_encode($session->all()), 'synthetic-secret-token'), 'no provider token in session');
    Auth::logout();
    $providers->history = [];
    $response = $controller->callback($request, $provider, $providers, $accounts);
    check(str_ends_with($response->getTargetUrl(), '/console/login') && ! Auth::check() && count($providers->history) === 0, 'callback replay rejected');
}

foreach (['state', 'provider', 'expired', 'denied', 'exchange'] as $failure) {
    $providers->history = [];
    $controller->redirect(requestFor('/auth/google'), 'google', $providers);
    $state = $session->get('state');
    if ($failure === 'expired') { $session->put('helm_oauth_attempt.expires', time() - 1); }
    $params = ['code' => 'synthetic-code', 'state' => $failure === 'state' ? 'invalid' : $state];
    if ($failure === 'denied') { $params['error'] = 'access_denied'; }
    $providers->responses = [new Response(400, [], '{"secret":"synthetic-secret-token"}')];
    $response = $controller->callback(requestFor('/callback', $params), $failure === 'provider' ? 'x' : 'google', $providers, $accounts);
    check(str_ends_with($response->getTargetUrl(), '/console/login') && ! Auth::check(), $failure.' fails closed');
    check($session->get('errors')->first('oauth') === 'Sign-in could not be completed. Please try again.', 'generic failure');
    check(! str_contains(json_encode($session->all()), 'synthetic-secret-token'), 'no failure response leak');
    check(! $session->has('state') && ! $session->has('code_verifier'), 'failure cleared attempt');
    if ($failure !== 'exchange') { check(count($providers->history) === 0, $failure.' no network call'); }
}
check(! in_array('access_token', Illuminate\Support\Facades\Schema::getColumnListing('oauth_identities')), 'schema has no token column');
try { $migration->down(); throw new LogicException('rollback unexpectedly allowed'); }
catch (RuntimeException $e) { check(str_contains($e->getMessage(), 'data-preserving rollback'), 'unsafe downgrade refused'); }
check(Tenant::count() === 5, 'rollback preserves tenants');
// Fresh installation uses the same additive path and has no implicit tenant.
Illuminate\Support\Facades\Schema::dropAllTables();
foreach (glob(__DIR__.'/../database/migrations/0001_*.php') as $path) { (require $path)->up(); }
$migration->up();
check(Tenant::count() === 0 && User::count() === 0 && OAuthIdentity::count() === 0, 'fresh migration empty');
$seedTenant = Tenant::create(['id' => (string) Str::uuid(), 'principal_id' => (string) Str::uuid(), 'name' => 'Fixture']);
$seedUser = User::create(['name' => 'Fixture', 'email' => null, 'tenant_id' => $seedTenant->id, 'password' => null]);
check($seedUser->tenant->name === 'Fixture', 'main fixture API');
$passed = true;
echo "OAuth offline checks passed: $count\n";
