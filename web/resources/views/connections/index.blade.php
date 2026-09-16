<x-layouts.console>
    <flux:main class="mx-auto w-full max-w-4xl space-y-6" role="main">
        <div class="flex items-center justify-between gap-4"><flux:heading size="xl">Your Vessels</flux:heading><flux:button href="{{ route('console') }}" variant="ghost" icon="arrow-left">Console</flux:button></div>
        <flux:modal.trigger name="connection-help"><flux:button variant="ghost" icon="question-mark-circle">Connection help</flux:button></flux:modal.trigger>
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
            <form class="space-y-4" method="post" action="{{ route('connections.store') }}">@csrf
                <flux:input name="name" label="Connection name" maxlength="100" required />
                <flux:textarea name="credential" label="Credential JSON" maxlength="16384" autocomplete="off" required />
                <flux:button type="submit" variant="primary">Verify and connect</flux:button>
            </form>
        </flux:card>
        <flux:modal name="connection-help" class="md:w-xl">
            <div class="space-y-6">
                <flux:heading size="lg">Connect your Vessel</flux:heading>
                <flux:text>Each tenant can connect up to 64 of their own Vessels. Helm Web connects to them; it does not host your agents.</flux:text>
                <div class="space-y-3">
                    <flux:heading>1. Make your Vessel reachable</flux:heading>
                    <flux:text>We recommend Cloudflare Tunnel to give your Vessel a public HTTPS hostname without opening an inbound router port.</flux:text>
                    <flux:text>Install cloudflared on the Vessel host, create a tunnel, and add a published application route such as vessel.example.com. Point its service at the loopback address of your Vessel’s authenticated public API—not the private supervisor listener. Keep Vessel authentication enabled.</flux:text>
                    <flux:text>Use a hostname covered by your Cloudflare certificate. The endpoint must support HTTPS/WSS on port 443, without an interactive browser login in front of the API.</flux:text>
                    <flux:link href="https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/get-started/create-remote-tunnel/" target="_blank" rel="noopener noreferrer">Cloudflare Tunnel setup guide</flux:link>
                </div>
                <flux:separator />
                <div class="space-y-3">
                    <flux:heading>2. Create a pairing invitation</flux:heading>
            <flux:text>Your tenant's pairing principal: <code>{{ $tenant->principal_id }}</code></flux:text>
            <flux:text>On your Vessel host, create an invitation for this principal and its public HTTPS endpoint:</flux:text>
            <flux:card size="sm"><pre class="overflow-x-auto"><flux:text inline class="font-mono">vessel pair-invite --directory /path/to/vessel/state \
  --endpoint https://your-vessel.example.com \
  --principal {{ $tenant->principal_id }} \
  --workspace /your/workspace \
  --rights catalogue,observe,history,execute,steer,decide,cancel \
  --output /private/invitation.json</flux:text></pre></flux:card>
                    <flux:text>Replace the example paths and hostname, then paste the generated invitation JSON into the pairing form.</flux:text>
                </div>
                <flux:separator />
                <div class="space-y-3">
                    <flux:heading>Already have a connection credential?</flux:heading>
                    <flux:text>Use the import form with credential JSON containing endpoint, vessel_id, grant_id and token. Use a dedicated grant. Credentials are encrypted server-side and never returned to the conversation client.</flux:text>
                    <flux:text>Localhost, private networks and private DNS answers are not supported. Other public HTTPS hosting works too; Cloudflare Tunnel is a recommendation, not a requirement.</flux:text>
                </div>
                <flux:modal.close><flux:button variant="primary">Got it</flux:button></flux:modal.close>
            </div>
        </flux:modal>
    </flux:main>
</x-layouts.console>
