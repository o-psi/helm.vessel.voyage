<div id="helm-client" class="h-dvh overflow-hidden" wire:ignore data-tenant-id="{{ $tenantId }}" data-ticket-url="{{ route('console.ticket', absolute: false) }}" data-socket-path="{{ config('helm.gateway_path') }}">
    <flux:header class="border-b border-zinc-200 bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-900">
        <flux:sidebar.toggle class="lg:hidden" icon="bars-2" />
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
    <flux:sidebar collapsible="mobile" sticky class="min-h-0 border-e border-zinc-200 bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-900" aria-label="Voyages">
            <flux:sidebar.header><flux:heading>Voyages</flux:heading><flux:sidebar.collapse class="lg:hidden" /></flux:sidebar.header>
            <flux:select id="vessel" label="Vessel">@foreach($vessels as $vessel)<flux:select.option value="{{ $vessel->id }}">{{ $vessel->name }}</flux:select.option>@endforeach</flux:select>
            @if($vessels->isEmpty())<flux:text>No Vessels connected. <flux:link href="{{ route('connections') }}">Add your first Vessel</flux:link>.</flux:text>@endif
            <flux:input id="voyage-search" type="search" icon="magnifying-glass" label="Find a voyage" placeholder="Search voyages" />
            <flux:button id="reconnect" variant="ghost" icon="arrow-path" class="w-full">Refresh connection</flux:button>
            <flux:sidebar.nav id="voyages" aria-label="Choose a voyage" />
            <flux:text size="sm" class="max-md:hidden">Open an existing voyage. Creating voyages, uploads and private terminals are available in native Helm.</flux:text>
    </flux:sidebar>
    <flux:main class="flex min-h-0 min-w-0 flex-col p-0!" role="main" aria-label="Conversation">
            <div class="flex flex-wrap items-center justify-between gap-3 border-b border-zinc-200 p-4 dark:border-zinc-700"><flux:heading id="voyage-title" size="lg" level="1">Choose a voyage</flux:heading><flux:badge id="connection-state" role="status">Connecting…</flux:badge></div>
            <flux:callout id="notice-panel" class="mx-4 mt-2" role="status" hidden><flux:callout.text id="notice" /></flux:callout>
            <div id="pending" class="space-y-2 px-4 pt-2 empty:hidden" aria-label="Unconfirmed commands"></div>
            <div id="conversation" class="min-h-0 flex-1 overflow-y-auto overscroll-contain p-4" tabindex="0" aria-label="Conversation messages">
                <flux:button id="earlier" class="mx-auto mb-4" hidden>Load earlier messages</flux:button>
                <div id="messages" class="mx-auto max-w-4xl space-y-4"></div>
                <flux:callout id="live-output" class="mx-auto mt-4 max-w-4xl" hidden>
                    <flux:callout.heading id="output-title">Live output · provisional</flux:callout.heading>
                    <flux:callout.text id="output-text" class="whitespace-pre-wrap break-words" />
                    <x-slot name="actions"><flux:button id="more-output" hidden>Load more output</flux:button></x-slot>
                </flux:callout>
            </div>
            <section id="decisions" class="max-h-[32dvh] space-y-3 overflow-y-auto px-4" aria-label="Pending decisions"></section>
            <form id="composer" class="space-y-3 border-t border-zinc-200 p-4 dark:border-zinc-700">
                <flux:textarea id="prompt" label="Message" rows="3" maxlength="65536" placeholder="Write a message…" class="max-h-[30dvh]" disabled />
                <div class="flex items-center gap-3"><flux:button id="send" type="submit" variant="primary" disabled>Send</flux:button><flux:button id="cancel" disabled>Cancel run</flux:button><flux:text size="sm" class="max-sm:hidden">Ctrl/⌘ + Enter to send</flux:text></div>
            </form>
    </flux:main>
    <flux:modal name="message-details" class="w-full md:max-w-3xl">
        <div class="space-y-4"><flux:heading size="lg">Tool / attachment details</flux:heading><pre class="overflow-auto"><flux:text inline id="message-details-content" class="whitespace-pre-wrap break-words font-mono" /></pre></div>
    </flux:modal>
        {{-- Blade renders Flux controls once; JS clones these for socket-driven content. --}}
        <template id="flux-card"><flux:card size="sm" class="space-y-3 break-words" /></template>
        <template id="flux-decision"><flux:callout variant="warning" class="decision"><flux:callout.heading data-decision-heading /><flux:callout.text data-decision-text class="whitespace-pre-wrap break-words" /><x-slot name="actions" class="flex-wrap"><div data-decision-actions class="flex flex-wrap items-center gap-2"></div></x-slot></flux:callout></template>
        <template id="flux-action"><flux:button size="sm"><span data-label></span></flux:button></template>
        <template id="flux-voyage"><flux:sidebar.item as="button"><span data-label></span></flux:sidebar.item></template>
        <template id="flux-heading"><flux:heading level="2"><span data-label></span></flux:heading></template>
        <template id="flux-text"><flux:text><span data-label></span></flux:text></template>
        <template id="flux-answer"><flux:input placeholder="Custom answer" aria-label="Custom answer" maxlength="4096" /></template>
        <template id="flux-details"><flux:modal.trigger name="message-details"><flux:button size="sm" variant="ghost">Tool / attachment details</flux:button></flux:modal.trigger></template>
        <template id="flux-link"><flux:link href="#" external rel="noopener noreferrer" /></template>
        <template id="flux-code"><flux:card size="sm"><pre class="overflow-x-auto"><flux:text inline class="font-mono" data-code /></pre></flux:card></template>
        <template id="flux-quote"><flux:callout><flux:callout.text data-quote /></flux:callout></template>
        <template id="flux-separator"><flux:separator /></template>
        @foreach(range(1, 4) as $level)
            <template id="flux-h{{ $level }}"><flux:heading :level="$level" :size="$level === 1 ? 'xl' : ($level === 2 ? 'lg' : null)" /></template>
        @endforeach
        <template id="flux-table"><flux:table><flux:table.columns><flux:table.column><span data-cell></span></flux:table.column></flux:table.columns><flux:table.rows><flux:table.row><flux:table.cell /></flux:table.row></flux:table.rows></flux:table></template>
</div>
