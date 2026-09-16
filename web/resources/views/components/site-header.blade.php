<flux:header class="border-b border-zinc-200 bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-900">
    <div class="mx-auto flex w-full max-w-7xl flex-wrap items-center gap-x-4 gap-y-2 py-3">
        <flux:brand href="{{ route('home') }}" name="Helm" aria-label="Helm, Vessel, and Voyage home">
            <x-slot name="logo" class="bg-accent text-accent-foreground font-bold">h.</x-slot>
        </flux:brand>

        <flux:navbar aria-label="Main navigation" class="order-last w-full sm:order-none sm:w-auto">
            <flux:navbar.item href="{{ route('products.helm') }}" :current="request()->routeIs('products.helm')">Helm</flux:navbar.item>
            <flux:navbar.item href="{{ route('products.vessel') }}" :current="request()->routeIs('products.vessel')">Vessel</flux:navbar.item>
            <flux:navbar.item href="{{ route('products.voyage') }}" :current="request()->routeIs('products.voyage')">Voyage</flux:navbar.item>
        </flux:navbar>

        <flux:spacer />
        <flux:dropdown x-data align="end">
            <flux:button variant="subtle" icon="sun" square aria-label="Preferred color scheme" />
            <flux:menu>
                <flux:menu.item icon="sun" x-on:click="$flux.appearance = 'light'">Light</flux:menu.item>
                <flux:menu.item icon="moon" x-on:click="$flux.appearance = 'dark'">Dark</flux:menu.item>
                <flux:menu.item icon="computer-desktop" x-on:click="$flux.appearance = 'system'">System</flux:menu.item>
            </flux:menu>
        </flux:dropdown>
        @if (\App\Services\ConsoleAccess::enabled())
            <flux:button size="sm" href="{{ \App\Services\ConsoleAccess::authenticated(request()) ? route('console') : route('console.login') }}">{{ \App\Services\ConsoleAccess::authenticated(request()) ? 'Open console' : 'Sign in' }}</flux:button>
        @else
            <flux:button size="sm" href="{{ route('products.helm') }}">Explore the suite</flux:button>
        @endif
    </div>
</flux:header>
