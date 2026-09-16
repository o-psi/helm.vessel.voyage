<x-layouts.public
    page-title="Voyage — Agent work that stays alive"
    page-description="Give every coding-agent job a durable session with its own conversation, tools, progress, and execution process."
    :share-image="false"
>
    <x-site-header />
    <main id="main" tabindex="-1" class="mx-auto max-w-7xl px-6 lg:px-8">
        <section class="space-y-8 py-12 sm:py-20">
            <div class="flex items-center gap-3 text-sm text-zinc-500 dark:text-zinc-400">
                <flux:link href="{{ route('home') }}">Product suite</flux:link>
                <span>/</span> Voyage
            </div>
            <div class="grid gap-8 lg:grid-cols-2 lg:items-end">
                <div class="space-y-4">
                    <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">Voyage / Persistent work</flux:text>
                    <flux:heading level="1" size="xl" class="text-4xl! leading-tight! tracking-tight sm:text-5xl!">Work that stays alive after you leave.</flux:heading>
                </div>
                <div class="space-y-6">
                    <flux:text class="text-base leading-relaxed">A Voyage is the work itself: one durable agent session with its own conversation, tools, progress, and process. Leave it running and return when you are ready.</flux:text>
                    <div class="flex flex-wrap gap-3">
                        <flux:button href="#benefits" variant="primary">Why Voyage <span aria-hidden="true">↓</span></flux:button>
                        <flux:link href="{{ route('products.helm') }}">Back to Helm <span aria-hidden="true">↗</span></flux:link>
                    </div>
                </div>
            </div>
        </section>
        <flux:card variant="soft">
            <div class="flex flex-wrap gap-x-8 gap-y-3 text-sm text-zinc-700 dark:text-zinc-300"><strong>One job.</strong>
                <span>One trusted history.</span>
                <span>Its own runtime.</span>
                <span>Ready to continue.</span>
            </div>
        </flux:card>
        <section id="benefits" class="space-y-8 py-12 sm:py-16">
            <div class="max-w-2xl space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">WHY VOYAGE</flux:text>
                <flux:heading level="2" size="xl">Keep the context. Keep the momentum.</flux:heading>
                <flux:text class="text-base leading-relaxed">Stop rebuilding the story every time you reconnect, change devices, or bring another machine into the job.</flux:text>
            </div>
            <div class="grid gap-8 md:grid-cols-3">
                <article class="space-y-3">
                    <span class="text-sm text-emerald-700 dark:text-emerald-400">01</span>
                    <flux:heading level="3" size="xl">Stay durable</flux:heading>
                    <flux:text class="text-base leading-relaxed">Conversation, progress, decisions, and cleanup obligations survive interface disconnects.</flux:text>
                </article>
                <article class="space-y-3">
                    <span class="text-sm text-emerald-700 dark:text-emerald-400">02</span>
                    <flux:heading level="3" size="xl">Reach across machines</flux:heading>
                    <flux:text class="text-base leading-relaxed">A Voyage can assign scoped work to participating Vessels while keeping one canonical owner.</flux:text>
                </article>
                <article class="space-y-3">
                    <span class="text-sm text-emerald-700 dark:text-emerald-400">03</span>
                    <flux:heading level="3" size="xl">Know what happened</flux:heading>
                    <flux:text class="text-base leading-relaxed">Persistent history and exact command handling make reconnects and retries reviewable.</flux:text>
                </article>
            </div>
        </section>
        <section class="grid gap-8 border-t border-zinc-200 py-12 md:grid-cols-2 dark:border-zinc-700">
            <div class="space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">INDEPENDENT BY DESIGN</flux:text>
                <flux:heading level="2" size="xl">One Voyage. One runtime.</flux:heading>
                <flux:text class="text-base leading-relaxed">Each session runs in its own process, so one failed job does not take every other Voyage down with it.</flux:text>
            </div>
            <div class="space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">ON THE HORIZON / PLANNED</flux:text>
                <flux:heading level="2" size="xl">Independent Voyages. Shared awareness.</flux:heading>
                <flux:text class="text-base leading-relaxed">Planned collaboration will let Voyages share relevant progress and coordinate under permissions you control.</flux:text>
            </div>
        </section>
        <section class="flex flex-wrap items-center justify-between gap-4 border-t border-zinc-200 py-8 dark:border-zinc-700">
            <span>See every Voyage from one place.</span>
            <flux:link href="{{ route('products.helm') }}">Meet Helm <span aria-hidden="true">↗</span></flux:link>
        </section>
    </main>
    <x-site-footer />
</x-layouts.public>
