                <flux:heading size="lg">Set up your first Vessel</flux:heading>
                <flux:text>Helm is this web console. A Vessel runs on your computer or server and manages your AI conversations, called voyages. Connecting gives this web account full access to that Vessel. Your computer must stay on while voyages run.</flux:text>
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
                    <flux:text>In a terminal on that same computer, replace the two example values below. “State” is the directory used when starting Vessel. Your web account ID is already filled in.</flux:text>
                    <flux:card size="sm"><pre class="overflow-x-auto select-all"><flux:text inline class="font-mono">STATE="/path/to/vessel/state"
ENDPOINT="https://vessel.example.com"

INVITE_DIR=$(mktemp -d)
vessel pair-invite --directory "$STATE" \
  --endpoint "$ENDPOINT" \
  --principal {{ $tenant->principal_id }} \
  --full-access \
  --output "$INVITE_DIR/invitation.json" &amp;&amp;
cat "$INVITE_DIR/invitation.json"</flux:text></pre></flux:card>
                </div>
                <div class="space-y-3">
                    <flux:heading>4. Paste and connect</flux:heading>
                    <flux:text>Choose a name like “My computer”, and paste the complete invitation printed in your terminal into “Invitation”. Click <strong>Connect Vessel</strong>, then close this dialog to start or open a voyage.</flux:text>
                    <flux:text>Use the invitation within 10 minutes. Keep it private—never paste it into a chat.</flux:text>
                </div>
                <flux:text>This connection can use all workspaces, voyages and provider accounts on your Vessel, including ones added later. Provider credentials stay on that computer. Only connect a web console you trust.</flux:text>
                <flux:text>Already connected with limited access? Make a new invitation with the command above and pair again; old connections are not automatically upgraded.</flux:text>
