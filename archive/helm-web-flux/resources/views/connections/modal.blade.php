<div x-data="{ adding: {{ $errors->any() && !$errors->has('confirm_disconnect') && $pairings->isEmpty() ? 'true' : 'false' }} }" x-init="@if(request()->boolean('manage-vessels') || session('manage_vessels')) $nextTick(() => $flux.modal('manage-vessels').show()) @endif">
    <flux:modal name="manage-vessels" class="w-full md:max-w-2xl" aria-labelledby="manage-vessels-title" data-connections-url="{{ route('connections') }}">
        <div class="space-y-5">
            <div class="pr-8">
                <flux:heading id="manage-vessels-title" size="xl"><span x-show="!adding">Your Vessels</span><span x-show="adding" x-cloak>Add a Vessel</span></flux:heading>
                <flux:text class="mt-1"><span x-show="!adding">The computers you’ve connected to Helm.</span><span x-show="adding" x-cloak>Connect a computer where your voyages will run.</span></flux:text>
            </div>
            @if(session('status')) <flux:callout role="status">{{ session('status') }}</flux:callout> @endif
            @error('connection') <flux:callout variant="danger" role="alert">{{ $message }}</flux:callout> @enderror
            @if($errors->any() && !$errors->has('connection')) <flux:callout variant="danger" role="alert">Check the supplied fields and try again. You’ll need to paste your invitation or credential again.</flux:callout> @endif

            <div x-show="!adding" class="space-y-5" data-vessel-list>
                <div class="grid grid-cols-1 items-start gap-3 sm:grid-cols-2" data-vessel-grid>
                    @forelse($vessels as $connection)
                        <flux:card x-data="{ details: false }" class="min-w-0 p-4" data-vessel-card>
                            <div class="mb-4 flex items-center justify-between gap-3">
                                <div class="flex size-10 shrink-0 items-center justify-center rounded-lg bg-zinc-100 dark:bg-zinc-700"><flux:icon.server-stack class="size-5 text-zinc-500" /></div>
                                <flux:button variant="ghost" size="sm" icon="ellipsis-horizontal" x-on:click="details = !details" x-bind:aria-expanded="details" aria-label="Options for {{ $connection->name }}" />
                            </div>
                            <flux:heading class="break-words">{{ $connection->name }}</flux:heading>
                            <flux:text size="sm" class="mt-1 break-all">{{ parse_url($connection->endpoint, PHP_URL_HOST) ?: $connection->endpoint }}</flux:text>
                            <div x-show="details" x-cloak class="mt-3 space-y-3 rounded-lg bg-zinc-50 p-4 dark:bg-zinc-900">
                                <dl class="space-y-2 text-sm">
                                    <div><dt class="font-medium">Address</dt><dd class="break-all text-zinc-500">{{ $connection->endpoint }}</dd></div>
                                    <div><dt class="font-medium">Vessel ID</dt><dd class="break-all font-mono text-zinc-500">{{ $connection->vessel_id }}</dd></div>
                                </dl>
                                <flux:separator />
                                <flux:text size="sm">Removing this connection won’t stop your voyages or disconnect other clients.</flux:text>
                                <form class="space-y-3" method="post" action="{{ route('connections.destroy', $connection->id) }}">
                                    @csrf @method('DELETE')
                                    <flux:checkbox name="confirm_disconnect" value="1" required label="Remove from my Vessels" />
                                    <flux:button type="submit" variant="danger" size="sm">Remove Vessel</flux:button>
                                </form>
                            </div>
                        </flux:card>
                    @empty
                        <div class="col-span-full space-y-2 py-8 text-center">
                            <flux:icon.server-stack class="mx-auto mb-3 size-8 text-zinc-400" />
                            <flux:heading>No Vessels yet</flux:heading>
                            <flux:text>Add your first computer to start a voyage.</flux:text>
                        </div>
                    @endforelse
                </div>
                @foreach($pairings as $pairing)
                    <flux:callout icon="clock">
                        <flux:callout.heading>{{ $pairing->name }} · not confirmed</flux:callout.heading>
                        <flux:callout.text>We haven’t heard back yet. Check this attempt before adding it again.</flux:callout.text>
                        <form class="mt-3" method="post" action="{{ route('connections.retry', $pairing->id) }}">
                            @csrf
                            <flux:button type="submit" size="sm">Check connection</flux:button>
                        </form>
                    </flux:callout>
                @endforeach
                <div class="flex items-center justify-between border-t border-zinc-200 pt-4 dark:border-zinc-700">
                    <flux:button variant="primary" icon="plus" x-on:click="adding = true; $nextTick(() => $refs.vesselName.focus())">Add Vessel</flux:button>
                    <flux:modal.close><flux:button variant="ghost">Done</flux:button></flux:modal.close>
                </div>
            </div>

            <div x-show="adding" x-cloak class="space-y-4" data-vessel-add>
                <form class="space-y-4" method="post" action="{{ route('connections.pair') }}">
                    @csrf
                    <flux:input x-ref="vesselName" name="name" label="Name" placeholder="e.g. My laptop" maxlength="100" required />
                    <flux:textarea name="invitation" label="Invitation" placeholder="Paste the invitation from your Vessel" rows="3" maxlength="16384" autocomplete="off" spellcheck="false" required />
                    <details class="space-y-4 text-sm">
                        <summary class="cursor-pointer font-medium">Where do I get an invitation?</summary>
                        @include('connections.setup')
                    </details>
                    <div class="flex items-center justify-between pt-2">
                        <flux:button variant="ghost" icon="arrow-left" x-on:click="adding = false">Back</flux:button>
                        <flux:button type="submit" variant="primary">Connect Vessel</flux:button>
                    </div>
                </form>
                <details @if($errors->any() && session('vessel_form') === 'import') open @endif class="space-y-4 border-t border-zinc-200 pt-4 text-sm dark:border-zinc-700">
                    <summary class="cursor-pointer text-zinc-500">Use an existing credential instead</summary>
                    <form class="space-y-4" method="post" action="{{ route('connections.store') }}">
                        @csrf
                        <flux:input name="name" label="Name" maxlength="100" required />
                        <flux:textarea name="credential" label="Connection credential" rows="3" maxlength="16384" autocomplete="off" spellcheck="false" required />
                        <flux:button type="submit" variant="primary">Connect Vessel</flux:button>
                    </form>
                </details>
            </div>
        </div>
    </flux:modal>
</div>
