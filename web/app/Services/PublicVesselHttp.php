<?php
namespace App\Services;

use RuntimeException;
use Symfony\Component\Process\Process;

/** Public-only HTTPS, with one bounded DNS resolution and connect-time pinning. */
class PublicVesselHttp {
    public static function origin(string $value): string {
        $parts = parse_url($value);
        if (!$parts || ($parts['scheme'] ?? '') !== 'https' || empty($parts['host'])
            || array_diff(array_keys($parts), ['scheme', 'host', 'port', 'path'])
            || (isset($parts['port']) && $parts['port'] !== 443)
            || !in_array($parts['path'] ?? '', ['', '/'], true)) throw new RuntimeException('Invalid HTTPS origin.');
        $host = $parts['host'];
        $ip = trim($host, '[]');
        if (!filter_var($ip, FILTER_VALIDATE_IP) && !preg_match('/^(?=.{1,253}$)[a-z0-9]+(?:[a-z0-9.-]*[a-z0-9])?$/D', $host)) throw new RuntimeException('Invalid host.');
        $origin = 'https://'.$host;
        if (!in_array($value, [$origin, $origin.'/', $origin.':443', $origin.':443/'], true)) throw new RuntimeException('Noncanonical origin.');
        return $origin;
    }
    private static function subnet(string $ip, string $network, int $bits): bool {
        $a = inet_pton($ip); $b = inet_pton($network);
        if ($a === false || $b === false || strlen($a) !== strlen($b)) return false;
        $bytes = intdiv($bits, 8); $rest = $bits % 8;
        return substr($a, 0, $bytes) === substr($b, 0, $bytes)
            && (!$rest || (ord($a[$bytes]) >> (8-$rest)) === (ord($b[$bytes]) >> (8-$rest)));
    }
    public static function publicAddress(string $ip): bool {
        if (!filter_var($ip, FILTER_VALIDATE_IP)) return false;
        if (str_contains($ip, ':')) {
            if (!self::subnet($ip, '2000::', 3)) return false;
            $excluded = ['2001::/23', '2001:db8::/32', '2002::/16', '3fff::/20'];
        } else $excluded = ['0.0.0.0/8','10.0.0.0/8','100.64.0.0/10','127.0.0.0/8','169.254.0.0/16','172.16.0.0/12','192.0.0.0/24','192.0.2.0/24','192.88.99.0/24','192.168.0.0/16','198.18.0.0/15','198.51.100.0/24','203.0.113.0/24','224.0.0.0/4','240.0.0.0/4'];
        foreach ($excluded as $range) { [$network, $bits] = explode('/', $range); if (self::subnet($ip, $network, (int) $bits)) return false; }
        return true;
    }
    protected function resolve(string $host): array {
        if (filter_var($host, FILTER_VALIDATE_IP)) return [$host];
        // PHP's blocking resolver has no per-call deadline. Isolate only DNS in a
        // bounded PHP child; no shell, Node service, or unbounded FPM lookup.
        $code = <<<'DNS'
$queue=[$argv[1]]; $seen=[]; $ips=[];
while ($queue) {
    $host=array_shift($queue);
    if (isset($seen[$host])) continue;
    if (count($seen)>=8) exit(1);
    $seen[$host]=true;
    $records=dns_get_record($host, DNS_A|DNS_AAAA|DNS_CNAME);
    if ($records===false) exit(1);
    foreach ($records as $r) {
        if (isset($r['ip'])) $ips[]=$r['ip'];
        if (isset($r['ipv6'])) $ips[]=$r['ipv6'];
        if (isset($r['target'])) $queue[]=$r['target'];
        if (count($ips)>256 || count($queue)>32) exit(1);
    }
}
echo json_encode(array_values(array_unique($ips)));
DNS;
        $process = new Process([PHP_BINDIR.'/php', '-r', $code, $host]);
        $process->setTimeout(5); $process->mustRun();
        return json_decode($process->getOutput(), true, flags: JSON_THROW_ON_ERROR);
    }
    public function post(string $origin, string $path, array $body, array $headers = []): array {
        $origin = self::origin($origin);
        if (!in_array($path, ['/v1/vessel/pair','/v1/vessel/command','/v1/vessel/browser-credentials'], true)) throw new RuntimeException('Invalid operation.');
        $host = trim(parse_url($origin, PHP_URL_HOST), '[]');
        $addresses = $this->resolve($host);
        if (!$addresses) throw new RuntimeException('No public address.');
        foreach ($addresses as $address) if (!self::publicAddress($address)) throw new RuntimeException('Non-public address.');
        $pin = str_contains($addresses[0], ':') ? '['.$addresses[0].']' : $addresses[0];
        $bytes = json_encode($body, JSON_THROW_ON_ERROR);
        if (strlen($bytes) > 16384) throw new RuntimeException('Request too large.');
        $lines = ['Content-Type: application/json', 'Accept: application/json', 'Connection: close'];
        foreach ($headers as $key => $value) {
            if (!in_array($key, ['Authorization','x-voyage-grant','x-voyage-vessel'], true) || !is_string($value) || !preg_match('/^[\x20-\x7e]{1,4096}$/D', $value)) throw new RuntimeException('Invalid header.');
            $lines[] = $key.': '.$value;
        }
        $curl = curl_init($origin.$path); $response = ''; $headerBytes = 0;
        try {
            curl_setopt_array($curl, [CURLOPT_POST=>true, CURLOPT_POSTFIELDS=>$bytes, CURLOPT_HTTPHEADER=>$lines,
                CURLOPT_PROXY=>'', CURLOPT_NOPROXY=>'*', CURLOPT_FOLLOWLOCATION=>false, CURLOPT_MAXREDIRS=>0,
                CURLOPT_PROTOCOLS=>CURLPROTO_HTTPS, CURLOPT_REDIR_PROTOCOLS=>CURLPROTO_HTTPS,
                CURLOPT_SSL_VERIFYPEER=>true, CURLOPT_SSL_VERIFYHOST=>2, CURLOPT_CONNECTTIMEOUT_MS=>3000,
                CURLOPT_TIMEOUT_MS=>5000, CURLOPT_FRESH_CONNECT=>true, CURLOPT_FORBID_REUSE=>true,
                CURLOPT_RESOLVE=>[parse_url($origin, PHP_URL_HOST).':443:'.$pin],
                CURLOPT_HEADERFUNCTION=>function ($handle, $line) use (&$headerBytes) { $headerBytes += strlen($line); return $headerBytes <= 16384 ? strlen($line) : 0; },
                CURLOPT_WRITEFUNCTION=>function ($handle, $chunk) use (&$response) { if (strlen($response)+strlen($chunk)>65536) return 0; $response .= $chunk; return strlen($chunk); },
            ]);
            if (!curl_exec($curl) || curl_getinfo($curl, CURLINFO_RESPONSE_CODE) !== 200
                || !preg_match('~^application/json(?:\s*;|$)~i', curl_getinfo($curl, CURLINFO_CONTENT_TYPE) ?? '')) throw new RuntimeException('Vessel unavailable.');
            $result = json_decode($response, true, 32, JSON_THROW_ON_ERROR);
            if (!is_array($result)) throw new RuntimeException('Invalid response.');
            return $result;
        } finally { curl_close($curl); }
    }
}
