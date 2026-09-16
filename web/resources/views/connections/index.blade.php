<x-layouts.console>
    <flux:main class="mx-auto w-full max-w-4xl space-y-6" role="main">
        <div class="flex items-center justify-between gap-4"><flux:heading size="xl">Your Vessels</flux:heading><flux:button href="{{ route('console') }}" variant="ghost" icon="arrow-left">Console</flux:button></div>
        <flux:modal.trigger name="connection-help"><flux:button variant="ghost" icon="question-mark-circle">Set up your first Vessel</flux:button></flux:modal.trigger>
        @if(session('status')) <flux:callout role="status">{{ session('status') }}</flux:callout> @endif
        @error('connection') <flux:callout variant="danger" role="alert">{{ $message }}</flux:callout> @enderror
        @if($errors->any()) <flux:callout variant="danger" role="alert">Check the supplied fields. Connection secrets are never echoed back.</flux:callout> @endif
        @forelse($connections as $connection)
            <flux:card class="space-y-4"><flux:heading size="lg">{{ $connection->name }}</flux:heading><flux:text>{{ $connection->endpoint }}</flux:text><flux:text>Vessel {{ $connection->vessel_id }}</flux:text>
                <form class="space-y-4" method="post" action="{{ route('connections.destroy',$connection->id) }}">@csrf @method('DELETE')<flux:button type="submit" variant="primary">Disconnect</flux:button></form>
            </flux:card>
        @empty <flux:text>No Vessels yet. Start with “Set up your first Vessel” above.</flux:text> @endforelse
        <flux:card class="space-y-4">
            <flux:heading size="lg">Pair a Vessel</flux:heading>
            <form class="space-y-4" method="post" action="{{ route('connections.pair') }}">@csrf
                <flux:input name="name" label="Connection name" placeholder="My computer" maxlength="100" required />
                <flux:textarea name="invitation" label="Invitation from step 3" maxlength="16384" autocomplete="off" required />
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
                <flux:heading size="lg">Set up your first Vessel</flux:heading>
                <flux:text>Helm is this web console. A Vessel runs on your computer or server and manages your AI conversations, called voyages. Your computer must stay on while they run.</flux:text>
                <div class="space-y-3">
                    <flux:heading>1. Install on your computer</flux:heading>
                    <flux:text>Open a terminal on a Linux computer you control. Follow the <flux:link href="https://github.com/o-psi/voyage/blob/main/docs/getting-started.md" target="_blank" rel="noopener noreferrer">first-time setup guide</flux:link> to install Helm, Vessel and Voyage and connect your AI provider account. Then return here.</flux:text>
                </div>
                <div class="space-y-3">
                    <flux:heading>2. Give it a web address</flux:heading>
                    <flux:text>Follow the <flux:link href="https://github.com/o-psi/voyage/blob/main/docs/process-access.md#scoped-remote-access" target="_blank" rel="noopener noreferrer">Vessel web access setup</flux:link> to start its connection service. Use <flux:link href="https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/get-started/create-remote-tunnel/" target="_blank" rel="noopener noreferrer">Cloudflare Tunnel</flux:link> to point your hostname (for example, vessel.example.com) to <code>http://127.0.0.1:8080</code> on that computer.</flux:text>
                    <flux:text>You’ll need a domain on Cloudflare for this option. Keep Vessel authentication on; don’t publish its private service or add a browser login in front. Other public HTTPS hosting works too; localhost and private addresses won’t work here.</flux:text>
                </div>
                <div class="space-y-3">
                    <flux:heading>3. Make an invitation</flux:heading>
                    <flux:text>In a terminal on that same computer, replace the three example values below. “State” is the directory used when starting Vessel; “workspace” is the folder you want to work in. Your web account ID is already filled in.</flux:text>
                    <flux:card size="sm"><pre class="overflow-x-auto select-all"><flux:text inline class="font-mono">STATE="/path/to/vessel/state"
WORKSPACE="/path/to/your/project"
ENDPOINT="https://vessel.example.com"

INVITE_DIR=$(mktemp -d)
vessel pair-invite --directory "$STATE" \
  --endpoint "$ENDPOINT" \
  --principal {{ $tenant->principal_id }} \
  --workspace "$WORKSPACE" \
  --rights catalogue,observe,history,execute,steer,decide,cancel \
  --output "$INVITE_DIR/invitation.json" &amp;&amp;
cat "$INVITE_DIR/invitation.json"</flux:text></pre></flux:card>
                </div>
                <div class="space-y-3">
                    <flux:heading>4. Paste and connect</flux:heading>
                    <flux:text>Close this help, choose a name like “My computer”, and paste the complete invitation printed in your terminal into “Invitation from step 3”. Click <strong>Pair Vessel</strong>, then <strong>Console</strong> to open an existing voyage.</flux:text>
                    <flux:text>Use the invitation within 10 minutes. Keep it private—never paste it into a chat.</flux:text>
                </div>
                <details class="space-y-3">
                    <summary class="cursor-pointer">Optional: create new voyages or use an existing credential</summary>
                    <flux:text>The command above connects existing voyages. To create new ones, also grant <code>create,account_use</code> in <code>--rights</code> and add <code>--accounts ACCOUNT_UUID</code> with the provider account’s ID. See <flux:link href="https://github.com/o-psi/voyage/blob/main/docs/provider-accounts.md" target="_blank" rel="noopener noreferrer">provider account setup</flux:link>. Provider credentials stay on your computer.</flux:text>
                    <flux:text>Already have connection credential JSON? Use “Import an existing connection credential” instead of the invitation form.</flux:text>
                </details>
                <flux:modal.close><flux:button variant="primary">Got it</flux:button></flux:modal.close>
            </div>
        </flux:modal>
    </flux:main>
</x-layouts.console>
