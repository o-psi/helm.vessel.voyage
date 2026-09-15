<?php

namespace App\Models;

use Illuminate\Database\Eloquent\Attributes\Fillable;
use Illuminate\Database\Eloquent\Model;
use Illuminate\Database\Eloquent\Relations\BelongsTo;

#[Fillable(['provider', 'subject', 'user_id'])]
class OAuthIdentity extends Model
{
    protected $table = 'oauth_identities';

    // No access/refresh tokens, raw provider profiles, or credentials belong here.
    public function user(): BelongsTo
    {
        return $this->belongsTo(User::class);
    }
}
