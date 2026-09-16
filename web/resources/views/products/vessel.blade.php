<x-layouts.public
    page-title="Vessel — Turn every machine into agent capacity"
    page-description="Operate local and remote machines for coding agents while credentials and execution policy stay on the machine doing the work."
    :share-image="false"
>
    <x-site-header />
    <main id="main" tabindex="-1" class="mx-auto max-w-7xl px-6 lg:px-8">
        <section class="space-y-8 py-12 sm:py-20">
            <div class="flex items-center gap-3 text-sm text-zinc-500 dark:text-zinc-400">
                <flux:link href="{{ route('home') }}">Product suite</flux:link>
                <span>/</span> Vessel
            </div>
            <div class="grid gap-8 lg:grid-cols-2 lg:items-end">
                <div class="space-y-4">
                    <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">Vessel / Machine fleet</flux:text>
                    <flux:heading level="1" size="xl" class="text-4xl! leading-tight! tracking-tight sm:text-5xl!">Turn every machine into agent capacity.</flux:heading>
                </div>
                <div class="space-y-6">
                    <flux:text class="text-base leading-relaxed">Vessel makes the computers you control available for serious agent work. Add a workstation or server, keep its authority local, and see exactly what it is carrying.</flux:text>
                    <div class="flex flex-wrap gap-3">
                        <flux:button href="#benefits" variant="primary">Why Vessel <span aria-hidden="true">↓</span></flux:button>
                        <flux:link href="{{ route('products.voyage') }}">Next: Voyage <span aria-hidden="true">↗</span></flux:link>
                    </div>
                </div>
            </div>
        </section>
        <flux:card variant="soft">
            <div class="flex flex-wrap gap-x-8 gap-y-3 text-sm text-zinc-700 dark:text-zinc-300"><strong>Your hardware.</strong>
                <span>Your credentials.</span>
                <span>Your access rules.</span>
                <span>More room to run.</span>
            </div>
        </flux:card>
        <section id="benefits" class="space-y-8 py-12 sm:py-16">
            <div class="max-w-2xl space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">WHY VESSEL</flux:text>
                <flux:heading level="2" size="xl">Use the fleet you already own.</flux:heading>
                <flux:text class="text-base leading-relaxed">Put spare compute to work without giving up control of where code, credentials, and tools live.</flux:text>
            </div>
            <div class="grid gap-8 md:grid-cols-3">
                <article class="space-y-3">
                    <span class="text-sm text-emerald-700 dark:text-emerald-400">01</span>
                    <flux:heading level="3" size="xl">Add machines</flux:heading>
                    <flux:text class="text-base leading-relaxed">Run voyages on local or remote Vessels and choose the right machine for each job.</flux:text>
                </article>
                <article class="space-y-3">
                    <span class="text-sm text-emerald-700 dark:text-emerald-400">02</span>
                    <flux:heading level="3" size="xl">Keep secrets local</flux:heading>
                    <flux:text class="text-base leading-relaxed">Provider credentials stay on the executing machine instead of moving through a central service.</flux:text>
                </article>
                <article class="space-y-3">
                    <span class="text-sm text-emerald-700 dark:text-emerald-400">03</span>
                    <flux:heading level="3" size="xl">Control access</flux:heading>
                    <flux:text class="text-base leading-relaxed">Authorize who can observe, steer, or run work while the executing machine enforces its own policy.</flux:text>
                </article>
            </div>
        </section>
        <section class="grid gap-8 border-t border-zinc-200 py-12 md:grid-cols-2 dark:border-zinc-700">
            <div class="space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">BUILT TO SUPERVISE</flux:text>
                <flux:heading level="2" size="xl">Every job gets its own process.</flux:heading>
                <flux:text class="text-base leading-relaxed">Vessel starts, observes, and reconnects you to independent Voyage runtimes instead of packing every agent loop into one fragile service.</flux:text>
            </div>
            <div class="space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">BUILT FOR REAL NETWORKS</flux:text>
                <flux:heading level="2" size="xl">Local when you’re local. Remote when you’re not.</flux:heading>
                <flux:text class="text-base leading-relaxed">Helm reaches Vessels through authorized local and remote routes while the work stays on its chosen host.</flux:text>
            </div>
        </section>
        <section class="flex flex-wrap items-center justify-between gap-4 border-t border-zinc-200 py-8 dark:border-zinc-700">
            <span>Give every job a life of its own.</span>
            <flux:link href="{{ route('products.voyage') }}">Meet Voyage <span aria-hidden="true">↗</span></flux:link>
        </section>
    </main>
    <x-site-footer />
</x-layouts.public>
