<?php

namespace App\Livewire;

use Livewire\Attributes\Layout;
use Livewire\Component;

#[Layout('components.layouts.public')]
class Landing extends Component
{
    public string $surface = 'terminal';

    public function selectSurface(string $surface): void
    {
        abort_unless(in_array($surface, ['terminal', 'web', 'mobile'], true), 422);
        $this->surface = $surface;
    }

    public function render()
    {
        return view('livewire.landing');
    }
}
