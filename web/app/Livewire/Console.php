<?php

namespace App\Livewire;

use App\Services\ConsoleAccess;
use Livewire\Attributes\Layout;
use Livewire\Component;

#[Layout('components.layouts.console')]
class Console extends Component
{
    public function boot(): void
    {
        abort_unless(ConsoleAccess::authenticated(request()), 401);
    }

    public function render()
    {
        return view('livewire.console', ['vessels' => \App\Models\VesselConnection::where('tenant_id', request()->user()->tenant_id)->get(), 'tenantId' => request()->user()->tenant_id]);
    }
}
