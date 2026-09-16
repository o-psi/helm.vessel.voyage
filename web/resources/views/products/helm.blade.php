<x-layouts.public
    page-title="Helm — One command center for every agent"
    page-description="See, steer, and return to coding-agent work across local and remote machines from one focused command center."
    :share-image="false"
>
    <x-site-header />
    <main id="main" tabindex="-1" class="mx-auto max-w-7xl px-6 lg:px-8">
        <section class="space-y-8 py-12 sm:py-20">
            <div class="flex items-center gap-3 text-sm text-zinc-500 dark:text-zinc-400">
                <flux:link href="{{ route('home') }}">Product suite</flux:link>
                <span>/</span> Helm
            </div>
            <div class="grid gap-8 lg:grid-cols-2 lg:items-end">
                <div class="space-y-4">
                    <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">Helm / Command center</flux:text>
                    <flux:heading level="1" size="xl" class="text-4xl! leading-tight! tracking-tight sm:text-5xl!">Stay in control without standing still.</flux:heading>
                </div>
                <div class="space-y-6">
                    <flux:text class="text-base leading-relaxed">Helm gives you one place to start work, watch it move, and step in when an agent needs you. Close the interface and the job keeps going.</flux:text>
                    <div class="flex flex-wrap gap-3">
                        <flux:button href="#benefits" variant="primary">Why Helm <span aria-hidden="true">↓</span></flux:button>
                        <flux:link href="{{ route('products.vessel') }}">Next: Vessel <span aria-hidden="true">↗</span></flux:link>
                    </div>
                </div>
            </div>
        </section>
        <flux:card variant="soft">
            <div class="flex flex-wrap gap-x-8 gap-y-3 text-sm text-zinc-700 dark:text-zinc-300"><strong>One view.</strong>
                <span>Every voyage.</span>
                <span>Every machine.</span>
                <span>No artificial connection cap.</span>
            </div>
        </flux:card>
        <section id="benefits" class="space-y-8 py-12 sm:py-16">
            <div class="max-w-2xl space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">WHY HELM</flux:text>
                <flux:heading level="2" size="xl">See the work. Steer the work.</flux:heading>
                <flux:text class="text-base leading-relaxed">Give every agent room to run without losing the ability to jump in at the right moment.</flux:text>
            </div>
            <div class="grid gap-8 md:grid-cols-3">
                <article class="space-y-3">
                    <span class="text-sm text-emerald-700 dark:text-emerald-400">01</span>
                    <flux:heading level="3" size="xl">Run more at once</flux:heading>
                    <flux:text class="text-base leading-relaxed">Move between independent voyages instead of waiting for one terminal session to finish.</flux:text>
                </article>
                <article class="space-y-3">
                    <span class="text-sm text-emerald-700 dark:text-emerald-400">02</span>
                    <flux:heading level="3" size="xl">Come back anytime</flux:heading>
                    <flux:text class="text-base leading-relaxed">Disconnect without cancelling accepted work. Reopen Helm and return to the same session.</flux:text>
                </article>
                <article class="space-y-3">
                    <span class="text-sm text-emerald-700 dark:text-emerald-400">03</span>
                    <flux:heading level="3" size="xl">Stay close to decisions</flux:heading>
                    <flux:text class="text-base leading-relaxed">See progress, answer approvals, and redirect work from one focused interface.</flux:text>
                </article>
            </div>
        </section>
        <section class="grid gap-8 border-t border-zinc-200 py-12 md:grid-cols-2 dark:border-zinc-700">
            <div class="space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">LINUX TERMINAL</flux:text>
                <flux:heading level="2" size="xl">Fast by keyboard.</flux:heading>
                <flux:text class="text-base leading-relaxed">Use the Linux CLI and TUI to connect to local and remote Vessels. The v1.0.0 download includes Helm, Vessel, Voyage, and the installer.</flux:text>
            </div>
            <div class="space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">HELM WEB</flux:text>
                <flux:heading level="2" size="xl">Your browser is a Helm, too.</flux:heading>
                <flux:text class="text-base leading-relaxed">Sign in to Helm Web to start voyages, follow progress, and steer work on your connected Vessels. Install Vessel and Voyage on a machine you control first, expose its authenticated HTTPS/WSS endpoint, then pair it with your web account. Helm Web does not supply agent compute.</flux:text>
                <flux:button href="{{ route('console') }}" variant="primary">Open Helm Web</flux:button>
                <flux:link href="https://github.com/o-psi/helm.vessel.voyage/blob/main/docs/helm-web.md#connect-your-vessels">Web connection setup</flux:link>
            </div>
        </section>
        <section class="flex flex-wrap items-center justify-between gap-4 border-t border-zinc-200 py-8 dark:border-zinc-700">
            <span>Give Helm more machines to command.</span>
            <flux:link href="{{ route('products.vessel') }}">Meet Vessel <span aria-hidden="true">↗</span></flux:link>
        </section>
    </main>
    <x-site-footer />
</x-layouts.public>
