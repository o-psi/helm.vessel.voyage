<?php

use App\Livewire\Landing;
use Illuminate\Support\Facades\Route;

Route::get('/', Landing::class)->name('home');
Route::view('/helm', 'products.helm')->name('products.helm');
Route::view('/vessel', 'products.vessel')->name('products.vessel');
Route::view('/voyage', 'products.voyage')->name('products.voyage');
