<x-layouts.console>
    <main class="connections-page">
        <header><h1>Your Vessels</h1><a href="{{ route('console') }}">Open console</a></header>
        <p>Connect as many as 64 of your own publicly reachable Vessels. Helm Web does not host your agents.</p>
        @if(session('status')) <p role="status">{{ session('status') }}</p> @endif
        @error('connection') <p role="alert">{{ $message }}</p> @enderror
        @if($errors->any()) <p role="alert">Check the supplied fields. Connection secrets are never echoed back.</p> @endif
        @forelse($connections as $connection)
            <article class="message"><h2>{{ $connection->name }}</h2><p>{{ $connection->endpoint }}</p><p>Vessel {{ $connection->vessel_id }}</p>
                <form method="post" action="{{ route('connections.destroy',$connection->id) }}">@csrf @method('DELETE')<button>Disconnect</button></form>
            </article>
        @empty <p>No Vessels yet. Your tenant starts empty.</p> @endforelse
        <section class="message">
            <h2>Pair a Vessel</h2>
            <p>Your tenant's pairing principal: <code>{{ $tenant->principal_id }}</code></p>
            <p>On your Vessel host, create an invitation for this principal and its public HTTPS endpoint:</p>
            <pre>vessel pair-invite --directory /path/to/vessel/state \
  --endpoint https://your-vessel.example.com \
  --principal {{ $tenant->principal_id }} \
  --workspace /your/workspace \
  --rights catalogue,observe,history,execute,steer,decide,cancel \
  --output /private/invitation.json</pre>
            <form method="post" action="{{ route('connections.pair') }}">@csrf
                <label>Connection name<input name="name" maxlength="100" required></label>
                <label>Private invitation JSON<textarea name="invitation" maxlength="16384" autocomplete="off" required></textarea></label>
                <button>Pair Vessel</button>
            </form>
        </section>
        @foreach($pairings as $pairing)
            <section class="message"><h2>{{ $pairing->name }} · pairing unconfirmed</h2><p>The original pairing command is retained. Retry only this attempt; it uses the same identity.</p>
                <form method="post" action="{{ route('connections.retry',$pairing->id) }}">@csrf<button>Check / retry original pairing</button></form>
            </section>
        @endforeach
        <details class="message"><summary>Import an existing connection credential</summary>
            <p>Paste a private Vessel credential JSON containing endpoint, vessel_id, grant_id and token. Use a dedicated grant. Tokens are encrypted server-side and never returned to the conversation client.</p>
            <form method="post" action="{{ route('connections.store') }}">@csrf
                <label>Connection name<input name="name" maxlength="100" required></label>
                <label>Credential JSON<textarea name="credential" maxlength="16384" autocomplete="off" required></textarea></label>
                <button>Verify and connect</button>
            </form>
        </details>
        <p>Only public HTTPS/WSS on port 443 is supported. Localhost, private network endpoints and private DNS answers are refused. The Vessel must already expose its authenticated public API through HTTPS.</p>
    </main>
</x-layouts.console>
