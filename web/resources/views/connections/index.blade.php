<x-layouts.console>
    <main class="mx-auto max-w-4xl space-y-6 px-6 py-10">
        <div class="flex items-center justify-between gap-4"><flux:heading size="xl">Your Vessels</flux:heading><flux:button href="{{ route('console') }}" variant="ghost" icon="arrow-left">Console</flux:button></div>
        <flux:text>Connect as many as 64 of your own publicly reachable Vessels. Helm Web does not host your agents.</flux:text>
        @if(session('status')) <flux:callout role="status">{{ session('status') }}</flux:callout> @endif
        @error('connection') <flux:callout variant="danger" role="alert">{{ $message }}</flux:callout> @enderror
        @if($errors->any()) <flux:callout variant="danger" role="alert">Check the supplied fields. Connection secrets are never echoed back.</flux:callout> @endif
        @forelse($connections as $connection)
            <flux:card class="space-y-4"><flux:heading size="lg">{{ $connection->name }}</flux:heading><flux:text>{{ $connection->endpoint }}</flux:text><flux:text>Vessel {{ $connection->vessel_id }}</flux:text>
                <form class="space-y-4" method="post" action="{{ route('connections.destroy',$connection->id) }}">@csrf @method('DELETE')<flux:button type="submit" variant="primary">Disconnect</flux:button></form>
            </flux:card>
        @empty <flux:text>No Vessels yet. Your tenant starts empty.</flux:text> @endforelse
        <flux:card class="space-y-4">
            <flux:heading size="lg">Pair a Vessel</flux:heading>
            <flux:text>Your tenant's pairing principal: <code>{{ $tenant->principal_id }}</code></flux:text>
            <flux:text>On your Vessel host, create an invitation for this principal and its public HTTPS endpoint:</flux:text>
            <pre class="overflow-x-auto rounded-lg bg-zinc-100 p-4 text-xs leading-6 dark:bg-zinc-900">vessel pair-invite --directory /path/to/vessel/state \
  --endpoint https://your-vessel.example.com \
  --principal {{ $tenant->principal_id }} \
  --workspace /your/workspace \
  --rights catalogue,observe,history,execute,steer,decide,cancel \
  --output /private/invitation.json</pre>
            <form class="space-y-4" method="post" action="{{ route('connections.pair') }}">@csrf
                <flux:input name="name" label="Connection name" maxlength="100" required />
                <flux:textarea name="invitation" label="Private invitation JSON" maxlength="16384" autocomplete="off" required />
                <flux:button type="submit" variant="primary">Pair Vessel</flux:button>
            </form>
        </flux:card>
        @foreach($pairings as $pairing)
            <flux:card class="space-y-4"><flux:heading size="lg">{{ $pairing->name }} · pairing unconfirmed</flux:heading><flux:text>The original pairing command is retained. Retry only this attempt; it uses the same identity.</flux:text>
                <form class="space-y-4" method="post" action="{{ route('connections.retry',$pairing->id) }}">@csrf<flux:button type="submit" variant="primary">Check / retry original pairing</flux:button></form>
            </flux:card>
        @endforeach
        <flux:card class="space-y-4"><flux:heading size="lg">Import an existing connection credential</flux:heading>
            <flux:text>Paste a private Vessel credential JSON containing endpoint, vessel_id, grant_id and token. Use a dedicated grant. Tokens are encrypted server-side and never returned to the conversation client.</flux:text>
            <form class="space-y-4" method="post" action="{{ route('connections.store') }}">@csrf
                <flux:input name="name" label="Connection name" maxlength="100" required />
                <flux:textarea name="credential" label="Credential JSON" maxlength="16384" autocomplete="off" required />
                <flux:button type="submit" variant="primary">Verify and connect</flux:button>
            </form>
        </flux:card>
        <flux:text>Only public HTTPS/WSS on port 443 is supported. Localhost, private network endpoints and private DNS answers are refused. The Vessel must already expose its authenticated public API through HTTPS.</flux:text>
    </main>
</x-layouts.console>
