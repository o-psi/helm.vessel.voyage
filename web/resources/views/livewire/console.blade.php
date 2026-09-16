<div class="flex h-dvh flex-col">
    <flux:header class="border-b border-zinc-200 bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-900">
        <flux:brand href="{{ route('console') }}" name="Helm" />
        <flux:badge class="max-sm:hidden">Your workspace</flux:badge>
        <flux:spacer />
        <flux:button href="{{ route('connections') }}" variant="ghost" icon="server-stack">Vessels</flux:button>
        <flux:dropdown>
            <flux:button variant="ghost" icon="sun" aria-label="Appearance" />
            <flux:menu><flux:menu.radio.group x-model="$flux.appearance"><flux:menu.radio value="light">Light</flux:menu.radio><flux:menu.radio value="dark">Dark</flux:menu.radio><flux:menu.radio value="system">System</flux:menu.radio></flux:menu.radio.group></flux:menu>
        </flux:dropdown>
        <form method="post" action="{{ route('console.logout') }}">@csrf <flux:button type="submit" variant="ghost">Sign out</flux:button></form>
    </flux:header>
    <main id="helm-client" class="grid min-h-0 flex-1 grid-rows-[auto_minmax(0,1fr)] md:grid-cols-[17rem_minmax(0,1fr)] md:grid-rows-1" wire:ignore data-tenant-id="{{ $tenantId }}" data-ticket-url="{{ route('console.ticket', absolute: false) }}" data-socket-path="{{ config('helm.gateway_path') }}">
        <aside class="max-h-[30dvh] space-y-4 overflow-y-auto border-b border-zinc-200 bg-zinc-50 p-4 md:max-h-none md:border-r md:border-b-0 dark:border-zinc-700 dark:bg-zinc-900" aria-label="Voyages">
            <flux:select id="vessel" label="Vessel">@foreach($vessels as $vessel)<option value="{{ $vessel->id }}">{{ $vessel->name }}</option>@endforeach</flux:select>
            @if($vessels->isEmpty())<flux:text>No Vessels connected. <flux:link href="{{ route('connections') }}">Add your first Vessel</flux:link>.</flux:text>@endif
            <flux:input id="voyage-search" type="search" icon="magnifying-glass" label="Find a voyage" placeholder="Search voyages" />
            <flux:button id="reconnect" variant="ghost" icon="arrow-path" class="w-full">Refresh connection</flux:button>
            <flux:navlist id="voyages" aria-label="Choose a voyage" />
            <flux:text size="sm" class="max-md:hidden">Open an existing voyage. Creating voyages, uploads and private terminals are available in native Helm.</flux:text>
        </aside>
        <section class="flex min-h-0 min-w-0 flex-col" aria-label="Conversation">
            <div class="flex flex-wrap items-center justify-between gap-3 border-b border-zinc-200 p-4 dark:border-zinc-700"><flux:heading id="voyage-title" size="lg">Choose a voyage</flux:heading><flux:badge id="connection-state" role="status">Connecting…</flux:badge></div>
            <flux:text id="notice" class="px-4 pt-2 empty:hidden" role="status" />
            <div id="pending" class="space-y-2 px-4 pt-2 empty:hidden" aria-label="Unconfirmed commands"></div>
            <div id="conversation" class="min-h-0 flex-1 overflow-y-auto overscroll-contain p-4" tabindex="0" aria-label="Conversation messages">
                <flux:button id="earlier" class="mx-auto mb-4" hidden>Load earlier messages</flux:button>
                <div id="messages" class="mx-auto max-w-4xl space-y-4"></div>
                <section id="live-output" class="mx-auto mt-4 max-w-4xl space-y-3 border-l-2 border-emerald-500 pl-4" hidden><flux:heading level="2">Live output · provisional</flux:heading><pre class="whitespace-pre-wrap break-words font-sans text-sm leading-7"></pre><flux:button id="more-output" hidden>Load more output</flux:button></section>
            </div>
            <section id="decisions" class="max-h-[32dvh] space-y-3 overflow-y-auto px-4" aria-label="Pending decisions"></section>
            <form id="composer" class="space-y-3 border-t border-zinc-200 p-4 dark:border-zinc-700">
                <flux:textarea id="prompt" label="Message" rows="3" maxlength="65536" placeholder="Write a message…" class="max-h-[30dvh]" disabled />
                <div class="flex items-center gap-3"><flux:button id="send" type="submit" variant="primary" disabled>Send</flux:button><flux:button id="cancel" disabled>Cancel run</flux:button><flux:text size="sm" class="max-sm:hidden">Ctrl/⌘ + Enter to send</flux:text></div>
            </form>
        </section>
        {{-- Blade renders Flux controls once; JS clones these for socket-driven content. --}}
        <template id="flux-card"><flux:card size="sm" class="space-y-3 break-words" /></template>
        <template id="flux-decision"><flux:callout variant="warning" class="decision space-y-3" /></template>
        <template id="flux-action"><flux:button size="sm"><span data-label></span></flux:button></template>
        <template id="flux-voyage"><flux:navlist.item as="button"><span data-label class="whitespace-normal break-words"></span></flux:navlist.item></template>
        <template id="flux-heading"><flux:heading level="2"><span data-label></span></flux:heading></template>
        <template id="flux-text"><flux:text><span data-label></span></flux:text></template>
        <template id="flux-answer"><flux:input placeholder="Custom answer" aria-label="Custom answer" maxlength="4096" /></template>
    </main>
</div>
