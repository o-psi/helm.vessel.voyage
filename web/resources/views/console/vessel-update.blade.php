<section id="setup-update" class="space-y-4" aria-label="Update Vessel" hidden>
    <flux:heading id="update-vessel-name" size="lg">Update Vessel</flux:heading>
    <flux:text id="update-current" class="setup-wrap" />
    <flux:text id="update-status" class="setup-wrap" role="status" aria-live="polite" />
    <div id="update-source" class="space-y-3">
        <flux:select id="update-channel" label="Update source">
            <flux:select.option value="stable">Latest stable release</flux:select.option>
            <flux:select.option value="nightly">Latest completed development build</flux:select.option>
        </flux:select>
        <flux:text size="sm">Development builds contain recent GitHub changes and may have unfinished features. The Vessel uses its own download access.</flux:text>
        <flux:button id="update-check" type="button" icon="arrow-down-tray">Check and prepare update</flux:button>
    </div>
    <div id="update-review" class="space-y-3" hidden>
        <flux:heading id="update-version">Prepared update</flux:heading>
        <flux:text id="update-description" class="break-all" />
        <flux:text id="update-services" class="setup-wrap" />
        <flux:text>Install this exact build on this Vessel? Its connection will briefly restart. Your draft, accounts and existing voyages are retained.</flux:text>
        <div class="flex flex-wrap gap-2">
            <flux:button id="update-approve" type="button" variant="primary">Update this Vessel</flux:button>
            <flux:button id="update-discard" type="button" variant="ghost">Not now</flux:button>
        </div>
    </div>
    <flux:button id="update-refresh" type="button" variant="ghost" icon="arrow-path" hidden>Check update status</flux:button>
    <flux:button id="update-continue" type="button" variant="primary" hidden>Continue setup</flux:button>
</section>
