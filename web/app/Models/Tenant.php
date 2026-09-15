<?php

namespace App\Models;

use Illuminate\Database\Eloquent\Attributes\Fillable;
use Illuminate\Database\Eloquent\Concerns\HasUuids;
use Illuminate\Database\Eloquent\Model;
use Illuminate\Database\Eloquent\Relations\HasMany;

#[Fillable(['id', 'principal_id', 'name'])]
class Tenant extends Model
{
    use HasUuids;

    public function uniqueIds(): array
    {
        return ['id', 'principal_id'];
    }

    public function users(): HasMany
    {
        return $this->hasMany(User::class);
    }
}
