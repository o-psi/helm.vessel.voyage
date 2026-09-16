<?php

namespace App\Livewire;

use Livewire\Attributes\Layout;
use Livewire\Component;

#[Layout('components.layouts.public')]
class Landing extends Component
{
    public function render()
    {
        return view('livewire.landing')->layoutData([
            'pageTitle' => 'Helm — Your agents, on your machines',
            'pageDescription' => 'Steer agents from your browser or terminal. Install Vessel and Voyage on your own Linux machine, then connect it to Helm Web.',
        ]);
    }
}
