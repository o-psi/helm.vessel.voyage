<?php

use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\Schema;

return new class extends Migration
{
    public function up(): void
    {
        Schema::create('tenants', function (Blueprint $table) {
            $table->uuid('id')->primary();
            $table->uuid('principal_id')->unique();
            $table->string('name')->default('Personal tenant');
            $table->timestamps();
        });
        Schema::table('users', function (Blueprint $table) {
            $table->dropUnique('users_email_unique');
            $table->string('email')->nullable()->change();
            $table->string('password')->nullable()->change();
            // Existing password users remain tenantless, with their data intact.
            $table->foreignUuid('tenant_id')->nullable()->unique()->constrained('tenants')->restrictOnDelete();
        });
        Schema::create('oauth_identities', function (Blueprint $table) {
            $table->id();
            $table->string('provider', 32);
            $subject = $table->string('subject', 191);
            // Opaque provider IDs must not merge by MySQL's default CI collation.
            if (in_array(Schema::getConnection()->getDriverName(), ['mysql', 'mariadb'], true)) {
                $subject->collation('utf8mb4_bin');
            }
            $table->foreignId('user_id')->constrained('users')->cascadeOnDelete();
            $table->timestamps();
            $table->unique(['provider', 'subject']);
        });
    }

    public function down(): void
    {
        // Restoring NOT NULL / UNIQUE email and password constraints would either
        // fail or destroy real accounts. Roll back code, not this additive schema.
        throw new RuntimeException('OAuth tenant migration requires an explicit data-preserving rollback plan.');
    }
};
