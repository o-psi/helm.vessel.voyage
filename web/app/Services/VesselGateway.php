<?php
namespace App\Services;
use Illuminate\Support\Facades\Http;
use RuntimeException;
final class VesselGateway {
    public function call(string $operation, array $payload): array {
        $base = config('helm.gateway_admin_url');
        if (!preg_match('~^http://(?:127\.0\.0\.1|\[::1\]):[0-9]+$~D', $base)) throw new RuntimeException('Gateway unavailable.');
        try {
            $response = Http::withToken(config('helm.gateway_secret'))->timeout(15)->connectTimeout(3)
                ->withoutRedirecting()->acceptJson()->post($base.'/'.$operation, $payload);
            if (!$response->successful() || !is_array($response->json())) throw new RuntimeException();
            return $response->json();
        } catch (\Throwable) { throw new RuntimeException('Vessel could not be verified. Check its public endpoint and connection credentials.'); }
    }
    public static function endpoint(string $value): string {
        $value = rtrim($value, '/');
        $parts = parse_url($value);
        abort_unless($parts && ($parts['scheme'] ?? '') === 'https' && !empty($parts['host'])
            && !isset($parts['user']) && !isset($parts['pass']) && !isset($parts['query']) && !isset($parts['fragment'])
            && empty($parts['path']) && (!isset($parts['port']) || $parts['port'] === 443), 422, 'Use a public HTTPS origin without a path.');
        return $value;
    }
}
