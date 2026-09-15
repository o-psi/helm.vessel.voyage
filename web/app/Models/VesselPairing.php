<?php
namespace App\Models;
use Illuminate\Database\Eloquent\Model;
use Illuminate\Database\Eloquent\Concerns\HasUuids;
class VesselPairing extends Model {
    use HasUuids;
    protected $guarded = [];
    protected $hidden = ['request'];
    protected function casts(): array { return ['request' => 'encrypted:array']; }
}
