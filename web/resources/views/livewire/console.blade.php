<div id="helm-client" class="h-dvh overflow-hidden" wire:ignore data-tenant-id="{{ $tenantId }}" data-vessels="{{ $vessels->map(fn ($vessel) => ['id' => $vessel->id, 'name' => $vessel->name, 'vessel_id' => $vessel->vessel_id])->values()->toJson() }}" data-ticket-url="{{ route('console.ticket', absolute: false) }}">
    <flux:sidebar collapsible="mobile" sticky class="min-h-0 border-e border-zinc-200 bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-900" aria-label="Voyages">
            <flux:sidebar.header><flux:heading>Voyages</flux:heading><flux:sidebar.collapse class="lg:hidden" /></flux:sidebar.header>

            @if($vessels->isEmpty())<flux:text>No Vessels connected. <flux:link href="{{ route('connections') }}">Add your first Vessel</flux:link>.</flux:text>@endif
            <flux:modal.trigger name="voyage-settings"><flux:button id="new-voyage" variant="primary" icon="plus" class="w-full">New voyage</flux:button></flux:modal.trigger>
            <flux:input id="voyage-search" type="search" icon="magnifying-glass" label="Find a voyage" placeholder="Voyage or Vessel…" />
            <flux:button id="reconnect" variant="ghost" icon="arrow-path" class="w-full">Refresh connections</flux:button>
            <flux:text id="fleet-state" size="sm" role="status" />
            <div id="vessel-statuses" class="space-y-2" role="status"></div>
            <flux:sidebar.nav id="voyages" aria-label="Choose a voyage" />
            <flux:text id="voyage-empty" size="sm" hidden />
            <div id="pending-creations" class="space-y-2" aria-label="Unconfirmed voyage creation"></div>
<div class="flex items-center justify-between">
        <form method="post" action="{{ route('console.logout') }}">@csrf <flux:button type="submit" variant="ghost">Sign out</flux:button></form>
</div><flux:text id="connection-state" size="sm" role="status">Connecting…</flux:text><flux:button href="{{ route('connections') }}" variant="ghost" icon="server-stack">Vessels</flux:button>
    </flux:sidebar>
    <flux:main class="flex min-h-0 min-w-0 flex-col p-0!" role="main" aria-label="Conversation">
            <div class="sr-only"><flux:heading id="voyage-title" level="1">Choose a voyage</flux:heading><flux:text id="voyage-vessel" hidden /></div>
            <flux:sidebar.toggle class="fixed start-3 top-3 z-20 lg:hidden" icon="bars-2" aria-label="Open voyage navigation" />
            <flux:callout id="notice-panel" class="mx-4 mt-2" role="status" hidden><flux:callout.text id="notice" /></flux:callout>
            <div id="pending" class="space-y-2 px-4 pt-2 empty:hidden" aria-label="Unconfirmed commands"></div>
            <div id="conversation" class="min-h-0 flex-1 overflow-y-auto overscroll-contain px-5 pb-8 pt-14 sm:px-8 lg:pt-10" tabindex="0" aria-label="Conversation messages">
                <flux:button id="earlier" class="mx-auto mb-4" hidden>Load earlier messages</flux:button>
                <flux:text id="conversation-empty" class="mx-auto max-w-4xl">Choose a voyage from any connected Vessel.</flux:text>
                <div id="messages" class="mx-auto max-w-3xl space-y-8"></div>
                <div id="live-previews" class="mx-auto max-w-3xl" aria-label="Provisional tool calls and provider reasoning" hidden></div>
                <flux:callout id="live-output" class="mx-auto mt-8 max-w-3xl border-0! bg-transparent! p-0! shadow-none!" hidden>
                    <flux:callout.heading id="output-title">Live output · provisional</flux:callout.heading>
                    <flux:callout.text id="output-text" class="whitespace-pre-wrap break-words" />
                    <x-slot name="actions"><flux:button id="more-output" hidden>Load more output</flux:button></x-slot>
                </flux:callout>
            </div>
            <section id="decisions" class="max-h-[32dvh] space-y-3 overflow-y-auto px-4" aria-label="Pending decisions"></section>
            <form id="composer" class="mx-auto w-full max-w-5xl px-4 pb-4 pt-2">
                <flux:composer id="prompt" submit="enter" label="Message" label:sr-only rows="3" max-rows="8" placeholder="Ask anything…" class="rounded-3xl! p-3! shadow-sm" disabled>
                    <x-slot name="actionsLeading" class="col-span-3! min-w-0 overflow-x-auto">
                        <div class="flex w-max flex-nowrap items-center gap-1 whitespace-nowrap [&>ui-dropdown]:shrink-0 sm:gap-2">
                            <flux:dropdown position="top" align="start">
                                <flux:button id="change-inference" type="button" size="sm" variant="ghost" icon:trailing="chevron-down" aria-label="Change model" disabled><span id="composer-model" class="max-w-48 truncate">Account &amp; model</span></flux:button>
                                <flux:popover id="edit-popover" class="w-80 max-w-[calc(100vw-2rem)] max-h-[60dvh] overflow-y-auto">
                                    <div id="edit-form" class="space-y-4">
                                        <flux:heading id="edit-title">Model</flux:heading>
                                        <flux:text id="edit-description" />
                                        <div hidden><flux:select id="edit-vessel" /><flux:select id="edit-workspace" /></div>
                                        <div id="edit-account-section" hidden><flux:select id="edit-account" variant="listbox" searchable label="Provider account" /><div class="mt-3 space-y-2"><flux:heading size="sm">Account usage</flux:heading><flux:text id="account-usage" class="whitespace-pre-line" role="status">Not loaded</flux:text><flux:button id="account-usage-refresh" type="button" size="sm" variant="ghost" icon="arrow-path">Refresh usage</flux:button></div></div>
                                        <div id="edit-model-section"><flux:select id="edit-model" variant="listbox" searchable label="Model" /></div>
                                        <div hidden><flux:slider id="edit-reasoning" label="Reasoning" min="0" max="1" step="1" /><flux:text id="edit-reasoning-value" size="sm" aria-live="polite">Provider default</flux:text></div>
                                        <div id="edit-service-section" hidden><flux:radio.group id="edit-service" label="Service tier" variant="pills" /></div>
                                        <flux:text id="edit-status" role="status" />
                                        <div class="flex flex-wrap gap-2"><flux:button id="edit-retry" type="button" size="sm">Reload</flux:button><flux:button id="edit-close" type="button" size="sm" variant="ghost">Cancel</flux:button><flux:button id="edit-save" type="button" size="sm" variant="primary" disabled>Apply</flux:button></div>
                                    </div>
                                </flux:popover>
                            </flux:dropdown>
                            <flux:dropdown position="top" align="start">
                                <flux:button id="change-account" type="button" size="sm" variant="ghost" icon="user-circle" icon:trailing="chevron-down" aria-label="Change account" disabled><span id="composer-account" class="max-w-40 truncate">Account</span></flux:button>
                                <flux:popover id="account-popover" class="w-80 max-w-[calc(100vw-2rem)] max-h-[60dvh] overflow-y-auto" />
                            </flux:dropdown>
                            <flux:dropdown position="top" align="start">
                                <flux:button id="change-service" type="button" size="sm" variant="ghost" icon:trailing="chevron-down" aria-label="Change service tier" disabled><span id="composer-service">Service</span></flux:button>
                                <flux:popover id="service-popover" class="w-72 max-w-[calc(100vw-2rem)]" />
                            </flux:dropdown>
                            <flux:dropdown position="top" align="start">
                                <flux:button id="change-reasoning" type="button" size="sm" variant="ghost" icon:trailing="chevron-down" disabled><span id="composer-reasoning">Default</span></flux:button>
                                <flux:popover id="reasoning-popover" class="w-64 max-w-[calc(100vw-2rem)] space-y-3">
                                    <div class="space-y-3"><flux:slider id="quick-reasoning" label="Reasoning" min="0" max="1" step="1" /><flux:text id="quick-reasoning-value" size="sm" aria-live="polite">Provider default</flux:text></div>
                                    <flux:text id="reasoning-status" role="status" />
                                    <flux:button id="reasoning-save" type="button" size="sm" variant="primary">Apply</flux:button>
                                </flux:popover>
                            </flux:dropdown>
                            <flux:dropdown position="top" align="start">
                                <flux:button id="change-access" type="button" size="sm" variant="ghost" icon="lock-open" icon:trailing="chevron-down" disabled><span id="composer-access">Access unknown</span></flux:button>
                                <flux:popover class="w-72 max-w-[calc(100vw-2rem)] space-y-3">
                                    <flux:select id="access-mode" label="Voyage access mode" disabled><flux:select.option value="">Access unknown</flux:select.option><flux:select.option value="read-only">Read only</flux:select.option><flux:select.option value="approval">Approval</flux:select.option><flux:select.option value="unrestricted">Full access</flux:select.option></flux:select>
                                    <flux:text size="sm">Read only restricts changes. Approval asks before actions that need permission. Full access runs without asking, within the executing host’s configured limits.</flux:text>
                                </flux:popover>
                            </flux:dropdown>
                        </div>
                    </x-slot>
                    <x-slot name="actionsTrailing" class="col-span-1! shrink-0 ps-2"><flux:button id="cancel" hidden type="button" size="sm" variant="subtle" icon="stop" aria-label="Cancel run" title="Cancel run" class="rounded-full!" disabled /><flux:button id="send" type="submit" size="sm" variant="primary" icon="arrow-up" aria-label="Send" title="Send · Enter" class="rounded-full!" disabled /></x-slot>
                </flux:composer>
                <flux:text id="inference-summary" class="sr-only" />
            </form>
    </flux:main>
    <flux:modal name="voyage-settings" class="w-full md:max-w-xl" :dismissible="false">
        <form id="voyage-settings-form" class="space-y-5">
            <div><flux:heading id="settings-title" size="lg">New voyage</flux:heading><flux:text id="settings-description" class="mt-2">Choose where your voyage runs and which provider account it uses.</flux:text></div>
            <flux:select id="settings-vessel" label="Vessel" />
            <flux:select id="settings-workspace" label="Workspace" />
            <div id="settings-custom-workspace" hidden><flux:input id="settings-workspace-path" label="Folder on this Vessel" placeholder="/home/you/project" description="Enter an existing absolute folder path on the Vessel, not on your browser’s computer." /></div>
            <flux:select id="settings-account" variant="listbox" searchable label="Provider account" placeholder="Choose an account" description="Accounts belong to the selected Vessel. Credentials stay there." />
            <flux:select id="settings-model" variant="listbox" searchable label="Model" placeholder="Choose a model" />
            <div class="grid gap-4 sm:grid-cols-2"><div class="space-y-3"><flux:slider id="settings-reasoning" label="Reasoning" min="0" max="1" step="1" /><flux:text id="settings-reasoning-value" size="sm" aria-live="polite">Provider default</flux:text></div><flux:radio.group id="settings-service" label="Service tier" variant="pills" /></div>
            <flux:callout id="settings-notice" role="status"><flux:callout.text id="settings-status">Loading…</flux:callout.text></flux:callout>
            <div class="flex flex-wrap justify-end gap-3"><flux:button id="settings-retry" type="button" variant="ghost" icon="arrow-path">Reload choices</flux:button><flux:modal.close><flux:button id="settings-close" type="button" variant="ghost">Cancel</flux:button></flux:modal.close><flux:button id="settings-save" type="submit" variant="primary" disabled>Create voyage</flux:button></div>
        </form>
    </flux:modal>
    <flux:modal name="message-details" class="w-full md:max-w-3xl">
        <div class="space-y-4"><flux:heading size="lg">Tool / attachment details</flux:heading><pre class="overflow-auto"><flux:text inline id="message-details-content" class="whitespace-pre-wrap break-words font-mono" /></pre></div>
    </flux:modal>
        {{-- Blade renders Flux controls once; JS clones these for socket-driven content. --}}
        <template id="flux-service-option"><flux:radio value=""><span data-option-label></span></flux:radio></template>
        <template id="flux-card"><article class="min-w-0 space-y-3 break-words" /></template>
        <template id="thread-user"><article class="ms-auto w-fit max-w-[92%] space-y-3 rounded-3xl rounded-br-lg bg-zinc-100 px-5 py-4 text-zinc-800 dark:bg-zinc-800 dark:text-zinc-100" aria-label="Your message" /></template>
        <template id="thread-assistant"><article class="me-auto min-w-0 max-w-[96%] space-y-3 rounded-3xl rounded-bl-lg border border-zinc-200/70 bg-zinc-50 px-5 py-4 dark:border-zinc-700/60 dark:bg-zinc-900" aria-label="Assistant message"><span data-interrupted class="text-xs text-amber-600" hidden>Interrupted attempt</span></article></template>
        <template id="thread-tools"><section class="space-y-2 rounded-2xl border border-zinc-200/70 p-3 dark:border-zinc-800" data-tool-group aria-label="Tool activity"><div class="flex items-center gap-2 px-1"><flux:icon.command-line class="size-4 text-zinc-400" /><span data-tool-label class="min-w-0 flex-1 text-xs text-zinc-500"></span><flux:button data-expand-tools type="button" size="xs" variant="ghost">Expand all</flux:button></div><div data-tool-body class="divide-y divide-zinc-200/70 dark:divide-zinc-800"></div></section></template>
        <template id="thread-tool-entry"><details class="group" data-tool-entry><summary class="flex cursor-pointer list-none items-center gap-2 py-2 text-sm"><flux:icon.chevron-right class="size-3 text-zinc-400 group-open:rotate-90" /><span data-entry-label class="min-w-0 flex-1 line-clamp-2 [overflow-wrap:anywhere]"></span><time data-entry-time class="shrink-0 text-xs text-zinc-400"></time></summary><div data-entry-body class="max-h-96 overflow-auto rounded-lg bg-zinc-50 p-3 dark:bg-zinc-900"></div></details></template>
        <template id="flux-decision"><flux:callout variant="warning" class="decision"><flux:callout.heading data-decision-heading /><flux:callout.text data-decision-text class="whitespace-pre-wrap break-words" /><x-slot name="actions" class="flex-wrap"><div data-decision-actions class="flex flex-wrap items-center gap-2"></div></x-slot></flux:callout></template>
        <template id="flux-action"><flux:button size="sm"><span data-label></span></flux:button></template>
        <template id="flux-option"><flux:select.option /></template>
        <template id="flux-search-option"><flux:select.option variant="listbox"><span data-option-label></span></flux:select.option></template>
        <template id="flux-voyage"><flux:sidebar.item as="button"><span data-label></span><x-slot name="badge"><span data-vessel-label class="block max-w-24 truncate"></span></x-slot></flux:sidebar.item></template>
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
