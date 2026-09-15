<?php

use App\Livewire\Landing;
use Illuminate\Support\Facades\Route;

Route::get('/landing', Landing::class)->name('home');
Route::get('/', \App\Livewire\Console::class)
    ->middleware([\App\Http\Middleware\ConsoleOperator::class, \App\Http\Middleware\ConsoleHeaders::class])
    ->name('console');
Route::redirect('/console', '/');
Route::view('/helm', 'products.helm')->name('products.helm');
Route::view('/vessel', 'products.vessel')->name('products.vessel');
Route::view('/voyage', 'products.voyage')->name('products.voyage');

// The public product site remains available when the private console is disabled.
Route::prefix('console')->middleware(\App\Http\Middleware\ConsoleHeaders::class)->group(function () {
    Route::view('/login', 'console.login')->name('console.login');
    Route::post('/login', [\App\Http\Controllers\ConsoleAuthController::class, 'login']);
    Route::post('/logout', [\App\Http\Controllers\ConsoleAuthController::class, 'logout'])->name('console.logout');
    Route::middleware(\App\Http\Middleware\ConsoleOperator::class)->group(function () {
        Route::post('/ticket', [\App\Http\Controllers\ConsoleAuthController::class, 'ticket'])->middleware('throttle:12,1')->name('console.ticket');
    });
});
