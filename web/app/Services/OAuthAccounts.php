<?php

namespace App\Services;

use App\Models\OAuthIdentity;
use App\Models\Tenant;
use App\Models\User;
use Illuminate\Database\UniqueConstraintViolationException;
use Illuminate\Support\Facades\DB;
use Laravel\Socialite\Contracts\User as ProviderUser;
use RuntimeException;

class OAuthAccounts
{
    public function resolve(string $provider, ProviderUser $profile): User
    {
        $subject = (string) $profile->getId();
        if ($subject === '' || strlen($subject) > 191) {
            throw new RuntimeException('Invalid OAuth subject.');
        }
        // Provider subject is the only identity key. Email is untrusted display
        // data, never an account lookup or an automatic linking mechanism.
        $existing = OAuthIdentity::where('provider', $provider)->where('subject', $subject)->first();
        if ($existing) {
            return $this->assignedUser($existing);
        }
        try {
            return DB::transaction(function () use ($provider, $subject, $profile) {
                $tenant = Tenant::create();
                $email = $profile->getEmail();
                $user = User::create([
                    'tenant_id' => $tenant->id,
                    'name' => mb_substr((string) ($profile->getName() ?: $profile->getNickname() ?: 'Helm user'), 0, 255),
                    'email' => is_string($email) && strlen($email) <= 255 && filter_var($email, FILTER_VALIDATE_EMAIL) ? $email : null,
                    'password' => null,
                ]);
                OAuthIdentity::create(['provider' => $provider, 'subject' => $subject, 'user_id' => $user->id]);
                return $user;
            }, 3);
        } catch (UniqueConstraintViolationException $exception) {
            // A simultaneous first login may win the unique identity insert.
            // The losing transaction rolls back its user AND tenant before lookup.
            $identity = OAuthIdentity::where('provider', $provider)->where('subject', $subject)->first();
            if (! $identity) {
                throw $exception;
            }
            return $this->assignedUser($identity);
        }
    }

    private function assignedUser(OAuthIdentity $identity): User
    {
        $user = $identity->user;
        if (! $user || ! $user->tenant()->exists()) {
            throw new RuntimeException('OAuth account has no tenant.');
        }
        return $user;
    }
}
