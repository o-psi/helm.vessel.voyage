<?php
namespace App\Models;
use Illuminate\Database\Eloquent\Model;
use Illuminate\Database\Eloquent\Concerns\HasUuids;
class VesselConnection extends Model {
    use HasUuids;
    protected $guarded = [];
    protected $hidden = ['credential'];
    protected function casts(): array { return ['credential' => 'encrypted:array', 'revision' => 'integer']; }
}
