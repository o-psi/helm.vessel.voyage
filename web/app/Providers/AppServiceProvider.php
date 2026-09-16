<?php

namespace App\Providers;

use Illuminate\Support\ServiceProvider;

class AppServiceProvider extends ServiceProvider
{
    /**
     * Register any application services.
     */
    public function register(): void
    {
        //
    }

    /**
     * Bootstrap any application services.
     */
    public function boot(): void
    {
        // A full fleet renews up to 64 independent leases every 30 seconds.
        \Illuminate\Support\Facades\RateLimiter::for('console-tickets', function (\Illuminate\Http\Request $request) {
            $user = $request->user()?->id ?? $request->ip();
            $alias = $request->input('vessel');
            $vessel = hash('sha256', is_string($alias) ? $alias : 'invalid');
            return [
                \Illuminate\Cache\RateLimiting\Limit::perMinute(256)->by('fleet:'.$user),
                \Illuminate\Cache\RateLimiting\Limit::perMinute(30)->by('vessel:'.$user.':'.$vessel),
            ];
        });
        \Livewire\Livewire::addPersistentMiddleware([
            \App\Http\Middleware\ConsoleHeaders::class,
            \App\Http\Middleware\ConsoleOperator::class,
        ]);
    }
}
