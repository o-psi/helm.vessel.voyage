<?php
use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\Schema;
return new class extends Migration {
    public function up(): void {
        Schema::create('vessel_connections', function (Blueprint $table) {
            $table->uuid('id')->primary();
            $table->uuid('tenant_id')->index();
            $table->string('name', 100);
            $table->string('endpoint', 2048);
            $table->uuid('vessel_id');
            $table->text('credential');
            $table->unsignedBigInteger('revision')->default(1);
            $table->timestamps();
            $table->unique(['tenant_id', 'vessel_id']);
        });
        Schema::create('web_gateway_tickets', function (Blueprint $table) {
            $table->string('id', 64)->primary();
            $table->uuid('tenant_id')->index();
            $table->unsignedBigInteger('user_id');
            $table->uuid('connection_id');
            $table->unsignedBigInteger('connection_revision');
            $table->string('session_id');
            $table->string('subject', 64);
            $table->unsignedBigInteger('expires_at');
        });
        Schema::create('vessel_pairings', function (Blueprint $table) {
            $table->uuid('id')->primary();
            $table->uuid('tenant_id')->index();
            $table->string('name', 100);
            $table->text('request');
            $table->string('status')->default('pending');
            $table->timestamps();
        });
    }
    public function down(): void {
        Schema::dropIfExists('web_gateway_tickets');
        Schema::dropIfExists('vessel_pairings');
        Schema::dropIfExists('vessel_connections');
    }
};
