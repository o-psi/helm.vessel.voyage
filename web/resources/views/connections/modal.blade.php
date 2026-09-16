<div x-data x-init="@if(request()->boolean('manage-vessels') || session('manage_vessels')) $nextTick(() => $flux.modal('manage-vessels').show()) @endif">
    <flux:modal name="manage-vessels" class="w-full md:max-w-3xl" aria-labelledby="manage-vessels-title" data-connections-url="{{ route('connections') }}">
        <div class="space-y-6">
            <div class="pr-8">
                <flux:heading id="manage-vessels-title" size="xl">Manage Vessels</flux:heading>
                <flux:text class="mt-2">Connect the computers and servers where your voyages run. Provider credentials stay on each Vessel.</flux:text>
            </div>
            @if(session('status')) <flux:callout role="status">{{ session('status') }}</flux:callout> @endif
            @error('connection') <flux:callout variant="danger" role="alert">{{ $message }}</flux:callout> @enderror
            @if($errors->any()) <flux:callout variant="danger" role="alert">Check the supplied fields. For your security, paste the invitation or credential again; secrets are never echoed back.</flux:callout> @endif

            <section class="space-y-3" aria-labelledby="connected-vessels-title">
                <div class="flex items-center justify-between gap-3">
                    <flux:heading id="connected-vessels-title" size="lg">Saved connections</flux:heading>
                    <flux:badge>{{ $vessels->count() }} / 64</flux:badge>
                </div>
                <flux:text size="sm">Saved does not mean online. Select a Vessel in the console to connect and check its availability.</flux:text>
                @forelse($vessels as $connection)
                    <flux:card class="space-y-3">
                        <flux:heading>{{ $connection->name }}</flux:heading>
                        <flux:text class="break-all">{{ $connection->endpoint }}</flux:text>
                        <details class="space-y-3">
                            <summary class="cursor-pointer text-sm font-medium">Connection details &amp; disconnect</summary>
                            <dl class="space-y-2 text-sm">
                                <div><dt class="font-medium">Vessel ID</dt><dd class="break-all font-mono">{{ $connection->vessel_id }}</dd></div>
                                <div><dt class="font-medium">Last saved (UTC)</dt><dd>{{ $connection->updated_at?->utc()->format('Y-m-d H:i') ?? 'Unknown' }}</dd></div>
                            </dl>
                            <flux:text size="sm">Disconnect removes this web account’s saved connection, not the Vessel or its voyages. Already admitted work continues. Other clients retain access; revoke the grant on the Vessel to remove that access.</flux:text>
                            <form class="space-y-3" method="post" action="{{ route('connections.destroy', $connection->id) }}">
                                @csrf @method('DELETE')
                                <flux:checkbox name="confirm_disconnect" value="1" required label="Remove this saved connection" />
                                <flux:button type="submit" variant="danger" size="sm">Disconnect {{ $connection->name }}</flux:button>
                            </form>
                        </details>
                    </flux:card>
                @empty
                    <flux:callout icon="server-stack">
                        <flux:callout.heading>No Vessels connected yet</flux:callout.heading>
                        <flux:callout.text>Pair your first computer below. Need an invitation? Expand the setup guide to get started.</flux:callout.text>
                    </flux:callout>
                @endforelse
            </section>

            @if($pairings->isNotEmpty())
                <section class="space-y-3" aria-labelledby="pending-pairings-title">
                    <flux:heading id="pending-pairings-title" size="lg">Unconfirmed pairings</flux:heading>
                    @foreach($pairings as $pairing)
                        <flux:card class="space-y-3">
                            <flux:heading>{{ $pairing->name }}</flux:heading>
                            <flux:text>The result is not yet confirmed. Check this original attempt instead of creating a new one; its command identity is retained.</flux:text>
                            <form method="post" action="{{ route('connections.retry', $pairing->id) }}">
                                @csrf
                                <flux:button type="submit" variant="primary" size="sm">Check / retry original pairing</flux:button>
                            </form>
                        </flux:card>
                    @endforeach
                </section>
            @endif

            <flux:separator />
            <section class="space-y-4" aria-labelledby="pair-vessel-title">
                <flux:heading id="pair-vessel-title" size="lg">Pair a Vessel</flux:heading>
                <flux:text>Paste a private, unexpired invitation from your Vessel. Pairing an existing Vessel updates its saved name and credential.</flux:text>
                <details class="space-y-4 rounded-lg border border-zinc-200 p-4 dark:border-zinc-700">
                    <summary class="cursor-pointer font-medium">Set up your first Vessel</summary>
                    @include('connections.setup')
                </details>
                <form class="space-y-4" method="post" action="{{ route('connections.pair') }}">
                    @csrf
                    <flux:input name="name" label="Connection name" placeholder="My computer" maxlength="100" required />
                    <flux:textarea name="invitation" label="Invitation JSON" rows="4" maxlength="16384" autocomplete="off" spellcheck="false" required />
                    <flux:text size="sm">Invitations expire after 10 minutes. Keep them private—never paste them into a voyage.</flux:text>
                    <flux:button type="submit" variant="primary">Pair Vessel</flux:button>
                </form>
            </section>
            <details class="space-y-4 rounded-lg border border-zinc-200 p-4 dark:border-zinc-700">
                <summary class="cursor-pointer font-medium">Advanced: import an existing credential</summary>
                <flux:text>Already have connection credential JSON? Verify the Vessel identity and save it here. Only import credentials from a computer you trust.</flux:text>
                <form class="space-y-4" method="post" action="{{ route('connections.store') }}">
                    @csrf
                    <flux:input name="name" label="Connection name" maxlength="100" required />
                    <flux:textarea name="credential" label="Credential JSON" rows="4" maxlength="16384" autocomplete="off" spellcheck="false" required />
                    <flux:button type="submit" variant="primary">Verify and connect</flux:button>
                </form>
            </details>
            <div class="flex justify-end"><flux:modal.close><flux:button variant="ghost">Done</flux:button></flux:modal.close></div>
        </div>
    </flux:modal>
</div>
