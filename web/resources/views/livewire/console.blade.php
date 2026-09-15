<div class="console-shell">
    <header class="console-header">
        <strong>Helm</strong><span>Private console</span>
        <form method="post" action="{{ route('console.logout') }}">@csrf <flux:button type="submit" size="sm">Sign out</flux:button></form>
    </header>
    <main id="helm-client" wire:ignore data-ticket-url="{{ route('console.ticket', absolute: false) }}" data-socket-path="{{ config('helm.gateway_path') }}">
        <aside aria-label="Voyages">
            <label for="vessel">Vessel</label>
            <select id="vessel">@foreach($vessels as $vessel)<option value="{{ $vessel }}">{{ $vessel }}</option>@endforeach</select>
            <label for="voyage-search">Find a voyage</label><input id="voyage-search" type="search" placeholder="Search voyages">
            <button id="reconnect" type="button">Reconnect / refresh</button>
            <nav id="voyages" aria-label="Choose a voyage"></nav>
            <p class="hint">Open an existing voyage. Creating voyages, file uploads and private terminals remain available in native Helm.</p>
        </aside>
        <section class="workspace" aria-label="Conversation">
            <div class="conversation-heading"><h1 id="voyage-title">Choose a voyage</h1><span id="connection-state" role="status">Connecting…</span></div>
            <p id="notice" role="status"></p>
            <div id="pending" aria-label="Unconfirmed commands"></div>
            <div id="conversation" tabindex="0" aria-label="Conversation messages"><button id="earlier" hidden>Load earlier messages</button><div id="messages"></div><section id="live-output" hidden><h2>Live output · provisional</h2><pre></pre><button id="more-output" hidden>Load more output</button></section></div>
            <section id="decisions" aria-label="Pending decisions"></section>
            <form id="composer"><label for="prompt">Message</label><textarea id="prompt" rows="3" maxlength="65536" placeholder="Write a message…" disabled></textarea>
                <div class="composer-actions"><button id="send" type="submit" disabled>Send</button><button id="cancel" type="button" disabled>Cancel run</button><span>Ctrl/⌘ + Enter to send</span></div>
            </form>
        </section>
    </main>
</div>
