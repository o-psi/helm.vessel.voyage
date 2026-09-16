<?php
namespace App\Services;
use Illuminate\Support\Str;
use RuntimeException;

/** Compatibility name; all bootstrap operations now go directly to public HTTPS. */
final class VesselGateway {
    public function __construct(private PublicVesselHttp $http) {}
    public function call(string $operation, array $payload): array {
        try {
            if ($operation === 'pair') {
                foreach (['principal_id','invitation_id','command_id','vessel_id'] as $key) if (!Str::isUuid($payload[$key] ?? '')) throw new RuntimeException();
                if (!is_string($payload['code'] ?? null) || !preg_match('/^[\x21-\x7e]{1,4096}$/D', $payload['code'])) throw new RuntimeException();
                $response = $this->http->post(self::endpoint($payload['endpoint']), '/v1/vessel/pair',
                    ['protocol'=>1] + array_intersect_key($payload, array_flip(['principal_id','invitation_id','command_id','code'])), ['x-voyage-vessel'=>$payload['vessel_id']]);
            } else {
                $v = $payload['connection'];
                foreach (['grant_id','vessel_id'] as $key) if (!Str::isUuid($v[$key] ?? '')) throw new RuntimeException();
                if (!is_string($v['token'] ?? null) || !preg_match('/^[a-f0-9]{64}$/Di', $v['token'])) throw new RuntimeException();
                $url = $v['url'];
                if (!str_starts_with($url, 'wss://') || !str_ends_with($url, '/v1/vessel/socket')) throw new RuntimeException();
                $origin = self::endpoint('https://'.substr($url, 6, -strlen('/v1/vessel/socket')));
                $headers = ['Authorization'=>'Bearer '.$v['token'], 'x-voyage-grant'=>$v['grant_id'], 'x-voyage-vessel'=>$v['vessel_id']];
                if ($operation === 'browser-credentials') {
                    $response = $this->http->post($origin, '/v1/vessel/browser-credentials', ['origin'=>self::endpoint(config('app.url'))], $headers);
                    if (array_diff(array_keys($response), ['token','expires_at_ms','vessel_id']) || count($response)!==3
                        || !is_string($response['token'] ?? null) || !preg_match('/^[\x21-\x7e]{1,4096}$/D', $response['token'])
                        || ($response['vessel_id'] ?? null) !== $v['vessel_id'] || !is_int($response['expires_at_ms'] ?? null)
                        || $response['expires_at_ms'] <= now()->getTimestampMs() || $response['expires_at_ms'] > now()->getTimestampMs()+120000) throw new RuntimeException();
                    return $response + ['url'=>preg_replace('/^https:/', 'wss:', $origin).'/v1/vessel/browser-socket'];
                }
                if ($operation !== 'probe') throw new RuntimeException();
                $response = $this->http->post($origin, '/v1/vessel/command', ['protocol'=>1,'command'=>['op'=>'capabilities']], $headers);
            }
            if (($response['protocol'] ?? null)!==1 || !array_key_exists('error',$response) || $response['error']!==null
                || ($response['outcome_unknown'] ?? null)!==false || !is_array($response['result'] ?? null)) throw new RuntimeException();
            if ($operation === 'pair') return $response;
            $result = $response['result'];
            if (($result['protocol'] ?? null)!==1 || ($result['vessel_id'] ?? null)!==$v['vessel_id'] || !is_array($result['features'] ?? null)) throw new RuntimeException();
            return $result;
        } catch (\Throwable) { throw new RuntimeException('Vessel could not be verified. Check its public endpoint and connection credentials.'); }
    }
    public static function endpoint(string $value): string { return PublicVesselHttp::origin($value); }
}
