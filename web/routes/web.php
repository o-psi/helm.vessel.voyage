<?php
use App\Livewire\Landing;
use App\Http\Controllers\{ConsoleAuthController,OAuthController,VesselConnectionController,GatewayController};
use App\Http\Middleware\{ConsoleHeaders,ConsoleOperator};
use Illuminate\Support\Facades\Route;
Route::get('/landing', Landing::class)->name('home');
Route::view('/helm','products.helm')->name('products.helm');
Route::view('/vessel','products.vessel')->name('products.vessel');
Route::view('/voyage','products.voyage')->name('products.voyage');
Route::post('/console/gateway/authorize',[GatewayController::class,'authorizeTicket'])->withoutMiddleware([
    \Illuminate\Foundation\Http\Middleware\PreventRequestForgery::class,
    \Illuminate\Session\Middleware\StartSession::class,
    \Illuminate\View\Middleware\ShareErrorsFromSession::class,
]);
Route::middleware(ConsoleHeaders::class)->group(function () {
    Route::view('/console/login','console.login')->name('console.login');
    Route::get('/auth/{provider}',[OAuthController::class,'redirect'])->name('oauth.redirect')->middleware('throttle:20,1');
    Route::get('/auth/{provider}/callback',[OAuthController::class,'callback'])->name('oauth.callback')->middleware('throttle:20,1');
    Route::post('/console/logout',[ConsoleAuthController::class,'logout'])->name('console.logout');
});
Route::middleware([ConsoleOperator::class,ConsoleHeaders::class])->group(function () {
    Route::get('/',\App\Livewire\Console::class)->name('console');
    Route::post('/console/ticket',[ConsoleAuthController::class,'ticket'])->name('console.ticket')->middleware('throttle:console-tickets');
    Route::get('/connections',[VesselConnectionController::class,'index'])->name('connections');
    Route::post('/connections',[VesselConnectionController::class,'store'])->name('connections.store')->middleware('throttle:10,1');
    Route::post('/connections/pair',[VesselConnectionController::class,'pair'])->name('connections.pair')->middleware('throttle:10,1');
    Route::post('/connections/pair/{id}/retry',[VesselConnectionController::class,'retry'])->name('connections.retry')->middleware('throttle:10,1');
    Route::delete('/connections/{id}',[VesselConnectionController::class,'destroy'])->name('connections.destroy');
});
