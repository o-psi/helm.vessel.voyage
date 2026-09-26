<div id="helm-client" class="h-dvh overflow-hidden" wire:ignore data-tenant-id="{{ $tenantId }}" data-vessels="{{ $vessels->map(fn ($vessel) => ['id' => $vessel->id, 'name' => $vessel->name, 'vessel_id' => $vessel->vessel_id])->values()->toJson() }}" data-ticket-url="{{ route('console.ticket', absolute: false) }}">    <flux:sidebar collapsible="mobile" sticky class="min-h-0 border-e border-zinc-200 bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-900" aria-label="Voyages">
            <flux:sidebar.header><flux:heading>Voyages</flux:heading><flux:sidebar.collapse class="lg:hidden" /></flux:sidebar.header>

            @if($vessels->isEmpty())<flux:text>No Vessels connected. <flux:link href="{{ route('connections') }}">Add your first Vessel</flux:link>.</flux:text>@endif
            <flux:button id="new-voyage" variant="primary" icon="plus" class="w-full">New voyage</flux:button>
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
    <flux:main class="browser-workspace flex min-h-0 min-w-0 flex-col p-0!" role="main" aria-label="Conversation">
            <div class="browser-conversation">
            <div class="sr-only"><flux:heading id="voyage-title" level="1">Choose a voyage</flux:heading><flux:text id="voyage-vessel" hidden /></div>
            <flux:sidebar.toggle class="fixed start-3 top-3 z-20 lg:hidden" icon="bars-2" aria-label="Open voyage navigation" />
            <flux:callout id="notice-panel" class="mx-4 mt-2" role="status" hidden><flux:callout.text id="notice" /></flux:callout>
            <div id="pending" class="space-y-2 px-4 pt-2 empty:hidden" aria-label="Unconfirmed commands"></div>
            <flux:button id="host-browser-toggle" type="button" variant="ghost" aria-expanded="false" aria-controls="host-browser-panel" disabled>Browser<span id="host-browser-activity" aria-label="Browser activity in this voyage" hidden></span></flux:button>
            <div id="conversation" class="min-h-0 flex-1 overflow-y-auto overscroll-contain px-5 pb-8 pt-14 sm:px-8 lg:pt-10" tabindex="0" aria-label="Conversation messages">
                <flux:button id="earlier" class="mx-auto mb-4" hidden>Load earlier messages</flux:button>
                <flux:text id="conversation-empty" class="mx-auto max-w-4xl">Choose a voyage from any connected Vessel.</flux:text>
                <div id="messages" class="mx-auto max-w-3xl space-y-8"></div>
                <flux:callout id="run-failure" variant="danger" class="mx-auto mt-8 max-w-3xl" role="status" hidden>
                    <flux:callout.heading>Run failed</flux:callout.heading>
                    <flux:callout.text id="run-failure-detail" class="whitespace-pre-wrap break-words" />
                </flux:callout>
                <div id="live-previews" class="mx-auto max-w-3xl" aria-label="Provisional tool calls and provider reasoning" hidden></div>
                <flux:callout id="live-output" class="mx-auto mt-8 max-w-3xl border-0! bg-transparent! p-0! shadow-none!" hidden>
                    <flux:callout.heading id="output-title">Live output · provisional</flux:callout.heading>
                    <flux:callout.text id="output-text" class="whitespace-pre-wrap break-words" />
                    <x-slot name="actions"><flux:button id="more-output" hidden>Load more output</flux:button></x-slot>
                </flux:callout>
            </div>
            <section id="decisions" class="max-h-[32dvh] space-y-3 overflow-y-auto px-4" aria-label="Pending decisions"></section>
            <form id="composer" class="mx-auto w-full max-w-3xl px-3 pb-2 pt-2 sm:px-4 sm:pb-4">
                <flux:composer id="prompt" submit="enter" label="Message" label:sr-only rows="2" max-rows="8" placeholder="Ask anything…" class="rounded-3xl! p-3! shadow-sm">
                    <x-slot name="input"><flux:textarea rows="2" readonly class="border-0! bg-transparent! shadow-none! resize-none" /></x-slot>
                    <x-slot name="actionsLeading" class="col-span-3! min-w-0">
                        <div class="flex min-w-0 items-center gap-1 sm:gap-2">
                            <flux:button id="change-setup" type="button" size="sm" variant="ghost" icon="adjustments-horizontal" aria-label="Open voyage setup" aria-haspopup="dialog" aria-controls="setup-dialog"><span id="composer-setup-label" class="max-w-36 truncate sm:max-w-56">Setup</span></flux:button>
                            <flux:button id="attach-picture" type="button" size="sm" variant="ghost" icon="paper-clip" aria-label="Attach pictures" tooltip="Attach pictures" />
                        </div>
                        {{-- Existing status targets stay available while the settings controller moves into the flyout. --}}
                        <div hidden>
                            <flux:button id="change-location" type="button" disabled><span id="composer-location">Location</span></flux:button>
                            <flux:button id="change-inference" type="button" disabled><span id="composer-model">Profile</span></flux:button>
                            <flux:button id="change-account" type="button" disabled><span id="composer-account">Account</span></flux:button>
                            <flux:button id="change-service" type="button" disabled><span id="composer-service">Service</span></flux:button>
                            <flux:button id="change-reasoning" type="button" disabled><span id="composer-reasoning">Default</span></flux:button>
                            <flux:button id="change-access" type="button" disabled><span id="composer-access">Access unknown</span></flux:button>
                        </div>
                    </x-slot>
                    <x-slot name="actionsTrailing" class="col-span-1! shrink-0 ps-2"><flux:button id="cancel" hidden type="button" size="sm" variant="subtle" icon="stop" aria-label="Cancel run" title="Cancel run" class="rounded-full!" disabled /><flux:button id="send" type="submit" size="sm" variant="primary" icon="arrow-up" aria-label="Send" title="Send · Enter" class="rounded-full!" disabled /></x-slot>
                </flux:composer>
                <input id="picture-files" type="file" accept="image/png,image/jpeg,image/webp" multiple hidden aria-label="Choose pictures" />
                <flux:text id="inference-summary" class="sr-only" />
            </form>
            </div>
            <aside id="host-browser-panel" tabindex="-1" aria-labelledby="host-browser-title" hidden>
                <header class="browser-panel-heading"><div><h2 id="host-browser-title">Browser</h2><span id="host-browser-voyage"></span></div><div class="browser-panel-actions"><flux:button id="host-browser-expand" type="button" variant="ghost" aria-pressed="false">Expand browser</flux:button><flux:button id="host-browser-close" type="button" variant="ghost" aria-label="Close browser viewer" title="Close viewer; keep browser running">Close ×</flux:button></div></header>
                <div id="host-browser-content"></div>
            </aside>
    </flux:main>
    <flux:modal id="setup-dialog" name="voyage-setup" flyout :closable="false" :dismissible="false" :escapable="false" class="voyage-setup-dialog">
        <div id="edit-form" class="voyage-setup-shell">
            <header id="setup-header" class="voyage-setup-header">
                <flux:button id="setup-back" type="button" size="sm" variant="ghost" icon="arrow-left" aria-label="Back" hidden />
                <flux:heading id="setup-title" size="lg" class="min-w-0 flex-1 truncate">Setup</flux:heading>
                <flux:button id="setup-close" type="button" size="sm" variant="ghost" icon="x-mark" aria-label="Close setup" />
            </header>
            <div id="setup-content" class="voyage-setup-content" tabindex="0">
                <flux:heading id="edit-title" class="sr-only">Voyage setup</flux:heading>
                <section id="setup-overview" class="space-y-4" aria-label="Voyage setup">
                    <flux:text id="edit-description" class="setup-wrap">Choose the location, profile, access and reasoning for your next run.</flux:text>
                    <div class="space-y-2">
                        <button id="setup-location-open" type="button" class="setup-nav-row">
                            <flux:icon.folder class="size-5 shrink-0 text-zinc-500" />
                            <span class="min-w-0 flex-1"><strong>Location</strong><span id="setup-location-summary" class="setup-wrap">Choose a Vessel and workspace</span></span>
                            <flux:icon.chevron-right class="size-4 shrink-0 text-zinc-400" />
                        </button>
                        <button id="setup-profile-open" type="button" class="setup-nav-row">
                            <flux:icon.user-circle class="size-5 shrink-0 text-zinc-500" />
                            <span class="min-w-0 flex-1"><strong>Profile</strong><span id="setup-profile-summary" class="setup-wrap">Choose a profile</span></span>
                            <flux:icon.chevron-right class="size-4 shrink-0 text-zinc-400" />
                        </button>
                        <button id="setup-access-open" type="button" class="setup-nav-row">
                            <flux:icon.lock-open class="size-5 shrink-0 text-zinc-500" />
                            <span class="min-w-0 flex-1"><strong>Access</strong><span id="setup-access-summary" class="setup-wrap">Vessel default</span></span>
                            <flux:icon.chevron-right class="size-4 shrink-0 text-zinc-400" />
                        </button>
                        <button id="setup-reasoning-open" type="button" class="setup-nav-row">
                            <flux:icon.sparkles class="size-5 shrink-0 text-zinc-500" />
                            <span class="min-w-0 flex-1"><strong>Reasoning</strong><span id="setup-reasoning-summary" class="setup-wrap">Provider default</span></span>
                            <flux:icon.chevron-right class="size-4 shrink-0 text-zinc-400" />
                        </button>
                    </div>
                </section>
                <section id="setup-location" class="space-y-4" aria-label="Location" hidden>
                    <div id="edit-location-section" class="space-y-4">
                        <flux:select id="edit-vessel" label="Vessel" />
                        <flux:select id="edit-workspace" label="Workspace" />
                        <div id="edit-custom-workspace" hidden><flux:input id="edit-workspace-path" label="Folder on this Vessel" description="Existing absolute path on the Vessel, not your browser’s computer." /></div>
                    </div>
                </section>
                <section id="setup-profiles" class="space-y-4" aria-label="Profiles" hidden>
                    <div id="edit-profiles-section" class="space-y-3">
                        <flux:input id="setup-profile-search" type="search" size="sm" icon="magnifying-glass" label="Find a profile" placeholder="Search profiles…" />
                        <select id="edit-profile" aria-label="Selected profile" hidden></select>
                        <div id="setup-profile-list" class="setup-list" aria-label="Saved profiles"></div>
                        <flux:text id="edit-profile-summary" class="setup-wrap" />
                    </div>
                    <flux:button id="setup-manage-open" type="button" size="sm" variant="ghost" icon="cog-6-tooth">Manage profiles</flux:button>
                </section>
                <section id="setup-manage" class="space-y-4" aria-label="Manage profiles" hidden>
                    <flux:button id="setup-manage-back" type="button" size="sm" variant="ghost" icon="arrow-left">Profiles</flux:button>
                    <div id="setup-manage-profile-list" class="setup-list" aria-label="Profiles to manage"></div>
                    <div id="edit-profile-actions" class="flex flex-wrap gap-2">
                        <flux:button id="profile-new" type="button" size="sm">Create</flux:button>
                        <flux:button id="profile-edit" type="button" size="sm">Edit</flux:button>
                        <flux:button id="profile-duplicate" type="button" size="sm">Duplicate</flux:button>
                        <flux:button id="profile-default" type="button" size="sm">Make default</flux:button>
                        <flux:button id="profile-delete" type="button" size="sm">Delete</flux:button>
                    </div>
                    <flux:text size="sm" class="setup-wrap">Profile changes affect future selections. Existing voyages keep their settings.</flux:text>
                </section>
                <section id="setup-delete" class="space-y-4" aria-label="Delete profile" hidden>
                    <flux:heading size="lg">Delete profile?</flux:heading>
                    <flux:text class="setup-wrap">This removes <strong id="setup-delete-name"></strong> from saved profiles. Existing voyages keep their current settings.</flux:text>
                    <flux:button id="setup-delete-confirm" type="button" variant="danger">Delete profile</flux:button>
                </section>
                <section id="setup-editor" class="space-y-5" aria-label="Profile editor" hidden>
                    <div id="edit-profile-name-section" class="space-y-2" hidden>
                        <flux:input id="edit-profile-name" label="Profile name" maxlength="80" />
                        <flux:button id="profile-cancel-edit" type="button" size="sm" variant="ghost">Cancel profile edit</flux:button>
                    </div>
                    <div id="edit-account-section" class="space-y-3" hidden>
                        <select id="edit-account" aria-label="Selected provider account" hidden></select>
                        <div>
                            <span class="setup-field-label">Provider account</span>
                            <button id="setup-account-open" type="button" class="setup-picker-row"><span id="setup-account-value" class="min-w-0 flex-1 setup-wrap">Choose an account</span><flux:icon.chevron-right class="size-4 shrink-0" /></button>
                        </div>
                        <flux:button id="edit-enroll" type="button" size="sm" variant="ghost" icon="plus">Add ChatGPT account</flux:button>
                        <div class="space-y-2">
                            <flux:heading size="sm">Account usage</flux:heading>
                            <flux:text id="account-usage" class="whitespace-pre-line setup-wrap" role="status">Not loaded</flux:text>
                            <flux:button id="account-usage-refresh" type="button" size="sm" variant="ghost" icon="arrow-path">Refresh usage</flux:button>
                        </div>
                    </div>
                    <div id="edit-model-section" class="space-y-2" hidden>
                        <select id="edit-model" aria-label="Selected model" hidden></select>
                        <span class="setup-field-label">Model</span>
                        <button id="setup-model-open" type="button" class="setup-picker-row"><span id="setup-model-value" class="min-w-0 flex-1 setup-wrap">Choose a model</span><flux:icon.chevron-right class="size-4 shrink-0" /></button>
                    </div>
                    <details class="setup-advanced">
                        <summary>Advanced options</summary>
                        <div class="space-y-4 pt-4">
                            <div id="edit-reasoning-section" hidden><flux:slider id="edit-reasoning" label="Reasoning" min="0" max="1" step="1" /><flux:text id="edit-reasoning-value" size="sm" aria-live="polite">Provider default</flux:text></div>
                            <div id="edit-service-section" hidden><flux:select id="edit-service" label="Service tier" /></div>
                        </div>
                    </details>
                </section>
                <section id="setup-picker" class="space-y-4" aria-label="Choose an option" hidden>
                    <flux:input id="setup-picker-search" type="search" size="sm" icon="magnifying-glass" label="Find an option" placeholder="Search…" />
                    <div id="setup-picker-list" class="setup-list" aria-label="Available options"></div>
                </section>
                <section id="setup-enrollment" aria-label="Connect ChatGPT account" hidden></section>
                <section id="setup-access" class="space-y-4" aria-label="Access mode" hidden>
                    <flux:select id="access-mode" label="Voyage access mode" disabled>
                        <flux:select.option value="">Vessel default</flux:select.option>
                        <flux:select.option value="read-only">Read only</flux:select.option>
                        <flux:select.option value="approval">Approval</flux:select.option>
                        <flux:select.option value="unrestricted">Full access</flux:select.option>
                    </flux:select>
                    <flux:text size="sm" class="setup-wrap">Read only restricts changes. Approval asks before actions that need permission. Full access runs without asking, within the executing host’s configured limits.</flux:text>
                    <flux:text id="setup-access-status" class="setup-wrap" role="status" />
                </section>
                <section id="setup-reasoning" class="space-y-4" aria-label="Reasoning" hidden>
                    <flux:text size="sm" class="setup-wrap">Adjust reasoning for the next run without changing the saved profile.</flux:text>
                    <div id="reasoning-popover" class="space-y-3">
                        <flux:slider id="quick-reasoning" label="Reasoning" min="0" max="1" step="1" />
                        <flux:text id="quick-reasoning-value" size="sm" aria-live="polite">Provider default</flux:text>
                        <flux:text id="reasoning-status" class="setup-wrap" role="status" />
                        <flux:button id="reasoning-save" type="button" size="sm" variant="primary">Apply reasoning</flux:button>
                    </div>
                </section>
            </div>
            <footer id="setup-footer" class="voyage-setup-footer">
                <flux:text id="edit-status" class="setup-wrap" role="status" />
                <div class="flex flex-wrap items-center justify-end gap-2">
                    <flux:button id="edit-retry" type="button" size="sm" variant="ghost">Reload</flux:button>
                    <flux:button id="edit-close" type="button" size="sm" variant="ghost">Cancel</flux:button>
                    <flux:button id="edit-save" type="button" size="sm" variant="primary" disabled>Apply</flux:button>
                    <flux:button id="setup-done" type="button" size="sm" variant="primary">Done</flux:button>
                </div>
            </footer>
        </div>
    </flux:modal>
    <div hidden>@include('console.account-enrollment')</div>
    <flux:modal name="sidebar-action" class="w-full md:max-w-2xl" :dismissible="false">
        <form id="sidebar-action-form" class="space-y-4">
            <flux:heading id="sidebar-action-title" size="lg">Voyage action</flux:heading>
            <flux:text id="sidebar-action-target" class="break-all" />
            <flux:text id="sidebar-action-status" role="status" />
            <pre id="sidebar-action-details" class="max-h-96 overflow-auto whitespace-pre-wrap break-words" hidden></pre>
            <flux:field id="sidebar-name-field"><flux:label>Name</flux:label><flux:input id="sidebar-name" maxlength="256" /></flux:field>
            <flux:field id="sidebar-access-field"><flux:label>Access mode</flux:label><flux:select id="sidebar-access"><option value="read-only">Read only</option><option value="approval">Approval</option><option value="unrestricted">Full access</option></flux:select></flux:field>
            <flux:field id="sidebar-branch-field"><flux:label>Branch through</flux:label><flux:select id="sidebar-branch"><option value="">Full conversation</option></flux:select><flux:button id="sidebar-branch-more" type="button" variant="ghost" size="sm">Load more branch points</flux:button></flux:field>
            <flux:field id="sidebar-retain-field"><flux:label>Recent messages to retain in working context</flux:label><flux:input id="sidebar-retain" type="number" min="0" max="4294967295" value="128" /></flux:field>
            <flux:field id="sidebar-confirm-field"><flux:label id="sidebar-confirm-label">Confirmation</flux:label><flux:input id="sidebar-confirm" autocomplete="off" /></flux:field>
            <div class="flex justify-end gap-3"><flux:button id="sidebar-reconcile" type="button" variant="ghost">Check pending receipt</flux:button><flux:button id="sidebar-dismiss" type="button" variant="ghost">Close</flux:button><flux:button id="sidebar-submit" type="submit" variant="primary" disabled>Confirm</flux:button></div>
        </form>
    </flux:modal>
    <flux:modal name="message-details" class="w-full md:max-w-3xl">
        <div class="space-y-4"><flux:heading size="lg">Tool / attachment details</flux:heading><pre class="overflow-auto"><flux:text inline id="message-details-content" class="whitespace-pre-wrap break-words font-mono" /></pre></div>
    </flux:modal>
        {{-- Blade renders Flux controls once; JS clones these for socket-driven content. --}}
        <template id="flux-card"><article class="min-w-0 space-y-3 break-words" /></template>
        <template id="thread-user"><article class="ms-auto w-fit max-w-[92%] space-y-3 rounded-3xl rounded-br-lg bg-zinc-100 px-5 py-4 text-zinc-800 dark:bg-zinc-800 dark:text-zinc-100" aria-label="Your message" /></template>
        <template id="thread-assistant"><article class="me-auto min-w-0 max-w-[96%] space-y-3 rounded-3xl rounded-bl-lg border border-zinc-200/70 bg-zinc-50 px-5 py-4 dark:border-zinc-700/60 dark:bg-zinc-900" aria-label="Assistant message"><span data-interrupted class="text-xs text-amber-600" hidden>Interrupted attempt</span></article></template>
        <template id="thread-tools"><section class="space-y-2 rounded-2xl border border-zinc-200/70 p-3 dark:border-zinc-800" data-tool-group aria-label="Tool activity"><div class="flex items-center gap-2 px-1"><flux:icon.command-line class="size-4 text-zinc-400" /><span data-tool-label class="min-w-0 flex-1 text-xs text-zinc-500"></span><flux:button data-expand-tools type="button" size="xs" variant="ghost">Expand all</flux:button></div><div data-tool-body class="divide-y divide-zinc-200/70 dark:divide-zinc-800"></div></section></template>
        <template id="thread-tool-entry"><details class="group" data-tool-entry><summary class="flex cursor-pointer list-none items-center gap-2 py-2 text-sm"><flux:icon.chevron-right class="size-3 text-zinc-400 group-open:rotate-90" /><span data-entry-label class="min-w-0 flex-1 line-clamp-2 [overflow-wrap:anywhere]"></span><time data-entry-time class="shrink-0 text-xs text-zinc-400"></time></summary><div data-entry-body class="max-h-96 overflow-auto rounded-lg bg-zinc-50 p-3 dark:bg-zinc-900"></div></details></template>
        <template id="flux-decision"><flux:callout variant="warning" class="decision"><flux:callout.heading data-decision-heading /><flux:callout.text data-decision-text class="whitespace-pre-wrap break-words" /><x-slot name="actions" class="flex-wrap"><div data-decision-actions class="flex flex-wrap items-center gap-2"></div></x-slot></flux:callout></template>
        <template id="flux-action"><flux:button size="sm"><span data-label></span></flux:button></template>
        <template id="flux-option"><flux:select.option /></template>
        <template id="flux-search-option"><flux:select.option variant="listbox"><span data-option-label></span></flux:select.option></template>
        <template id="flux-voyage"><flux:context class="block min-w-0">
            <div data-voyage-row class="flex min-w-0 items-center gap-1">
                <flux:sidebar.item as="button" class="min-w-0 flex-1"><span data-label></span><x-slot name="badge"><span data-vessel-label class="block max-w-24 truncate"></span></x-slot></flux:sidebar.item>
                <flux:button data-voyage-actions type="button" size="sm" variant="ghost" icon="ellipsis-vertical" aria-label="Voyage actions" aria-haspopup="menu" />
            </div>
            <flux:menu aria-label="Voyage actions"><flux:menu.item data-voyage-action="access">Access modes</flux:menu.item><flux:menu.item data-voyage-action="rename">Rename</flux:menu.item><flux:menu.item data-voyage-action="archive">Archive / Restore</flux:menu.item><flux:menu.item data-voyage-action="branch">Branch</flux:menu.item><flux:menu.item data-voyage-action="cancel">Cancel run</flux:menu.item><flux:menu.item data-voyage-action="details">Details</flux:menu.item><flux:menu.item data-voyage-action="clear">Clear</flux:menu.item><flux:menu.item data-voyage-action="compact">Compact</flux:menu.item><flux:menu.item data-voyage-action="delete">Delete</flux:menu.item></flux:menu>
        </flux:context></template>
        <template id="flux-attachment-panel"><div data-attachment-context class="col-span-4 min-w-0" hidden>

            <div class="flex gap-2 overflow-x-auto px-1 pb-2" data-images hidden></div>
            <div data-upload-errors class="space-y-1" hidden></div>

        </div></template>
        <template id="flux-attachment-image"><div class="relative flex w-16 shrink-0 flex-col" data-picture-card><img class="h-16 w-16 rounded-xl object-cover" hidden /><span data-image-name class="sr-only"></span><flux:button type="button" size="xs" variant="filled" icon="x-mark" aria-label="Remove picture" tooltip="Remove picture" class="absolute! -right-1 -top-1" /></div></template>

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
    @isset($tenant)
        @include('connections.modal')
    @endisset
</div>
