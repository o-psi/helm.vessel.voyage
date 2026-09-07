<div>
    <header class="site-header shell">
        <a class="wordmark" href="/" aria-label="Helm, Vessel, and Voyage home"><span class="brand-mark" aria-hidden="true">h.</span> helm</a>
        <nav aria-label="Main navigation">
            <a href="#system">How it works</a>
            <a href="#surfaces">Work anywhere</a>
            <a href="#start" class="nav-cta">Follow the launch <span aria-hidden="true">↗</span></a>
        </nav>
    </header>

    <main id="main">
        <section class="hero shell">
            <div class="hero-copy">
                <div class="eyebrow"><span class="status-dot"></span> Your machines are ready for more</div>
                <h1>Put every<br>machine<br><em>to work.</em></h1>
                <p class="hero-description">Run coding agents across the computers you already own. Start more work, keep it moving when you disconnect, and steer it all from Helm.</p>
                <div class="hero-actions">
                    <flux:button href="#system" variant="primary" class="primary-button">See how it works <span aria-hidden="true">↗</span></flux:button>
                    <a href="#surfaces" class="text-link">Work from anywhere <span aria-hidden="true">↓</span></a>
                </div>
                <p class="release-note"><span class="status-dot"></span> Built for Linux first <span class="separator">/</span> Web and mobile are next</p>
            </div>

            <div class="system-map" aria-label="Architecture illustration: Helm connects to local and remote Vessels, which supervise independent voyages">
                <div class="map-topline"><span>YOUR AGENT FLEET</span><span>LIVE OVERVIEW</span></div>
                <div class="helm-node"><span class="node-symbol">h.</span><div><strong>Helm</strong><span>See everything. Steer anything.</span></div><span class="node-tag">YOU'RE HERE</span></div>
                <div class="map-trunk" aria-hidden="true"></div>
                <div class="vessel-columns">
                    <div class="vessel-group"><div class="vessel-label"><span class="status-dot"></span> Local Vessel</div><div class="voyage-node"><span>↗</span> Refactor API <small>RUNNING</small></div><div class="voyage-node"><span>↗</span> Review changes <small>READY</small></div><div class="machine-caption">YOUR WORKSTATION</div></div>
                    <div class="vessel-group"><div class="vessel-label"><span class="status-dot"></span> Remote Vessel</div><div class="voyage-node"><span>↗</span> Ship release <small>RUNNING</small></div><div class="voyage-node"><span>↗</span> Fix mobile UI <small>WAITING</small></div><div class="machine-caption">YOUR SERVER</div></div>
                </div>
                <div class="map-bottomline"><span>Four voyages. Two machines.</span><span>One place to steer.</span></div>
            </div>
        </section>

        <div class="principles"><div class="shell principles-inner"><span>No artificial connection caps</span><span>Work survives disconnects</span><span>Credentials stay on your machines</span><span>Local and remote execution</span></div></div>

        <section id="system" class="system-section shell section-pad">
            <div class="section-heading"><div class="eyebrow">01 / HOW IT WORKS</div><h2>More capacity.<br>Less babysitting.</h2><p>Helm, Vessel, and Voyage remove the friction between you, your agents, and the machines ready to run them.</p></div>
            <div class="component-grid">
                <article class="component-card"><div class="card-index">01 <span>YOUR COMMAND CENTER</span></div><h3>Helm<span>.</span></h3><p>See every session, jump between jobs, answer approvals, and redirect work without keeping a terminal tied up. One clear view across all your machines.</p><div class="card-footer">Stay in control <span aria-hidden="true">↗</span></div></article>
                <article class="component-card"><div class="card-index">02 <span>YOUR MACHINE FLEET</span></div><h3>Vessel<span>.</span></h3><p>Turn a workstation, server, or spare computer into agent capacity. Keep provider credentials where the work runs and decide exactly who gets access.</p><div class="card-footer">Use every machine <span aria-hidden="true">↗</span></div></article>
                <article class="component-card"><div class="card-index">03 <span>YOUR PERSISTENT WORK</span></div><h3>Voyage<span>.</span></h3><p>Give every job its own durable session. The conversation, tools, and progress stay alive after you close Helm—and a voyage can reach across participating Vessels.</p><div class="card-footer">Keep work moving <span aria-hidden="true">↗</span></div></article>
            </div>
            <div class="domain-line"><span>THREE PRODUCTS, BUILT TOGETHER</span><p><strong>helm</strong><i>.</i><strong>vessel</strong><i>.</i><strong>voyage</strong></p></div>
        </section>

        <section id="surfaces" class="surfaces-section section-pad">
            <div class="shell surface-grid">
                <div><div class="eyebrow">02 / WORK ANYWHERE</div><h2>Leave the desk.<br>Keep moving.</h2><p class="section-description">Start in the terminal. Pick up from the web. Check in from your phone. Every interface connects to the same work instead of starting over.</p>
                    <div class="surface-tabs" role="group" aria-label="Explore Helm interfaces">
                        @foreach (['terminal' => 'CLI / TUI', 'web' => 'Web', 'mobile' => 'Mobile'] as $key => $label)
                            <flux:button wire:click="selectSurface('{{ $key }}')" :variant="$surface === $key ? 'primary' : 'ghost'" aria-pressed="{{ $surface === $key ? 'true' : 'false' }}">{{ $label }}</flux:button>
                        @endforeach
                    </div>
                </div>
                <div class="surface-panel" aria-live="polite" wire:loading.attr="aria-busy">
                    @if ($surface === 'terminal')
                        <div class="panel-label"><span class="status-dot"></span> BUILT FOR LINUX FIRST</div><h3>Move fast from<br>your terminal.</h3><p>Launch jobs, switch sessions, inspect progress, and answer approvals with a keyboard-first interface designed for serious daily work.</p><div class="terminal-line"><span>$</span> helm <span class="cursor" aria-hidden="true"></span></div>
                    @elseif ($surface === 'web')
                        <div class="panel-label">COMING NEXT / PLANNED</div><h3>See the whole fleet<br>from your browser.</h3><p>Open Helm without opening a terminal. Watch active work, steer voyages, and manage the Vessels you trust from one planned web experience.</p><div class="surface-caption">YOUR BROWSER → HELM → ALL YOUR WORK</div>
                    @else
                        <div class="panel-label">ON THE HORIZON / PLANNED</div><h3>Your agents don’t stop<br>when you step away.</h3><p>Check progress, answer a question, or redirect a voyage from the planned mobile app—without exposing your machines directly to the internet.</p><div class="surface-caption">YOUR POCKET → HELM → ALL YOUR WORK</div>
                    @endif
                </div>
            </div>
        </section>

        <section class="collaboration-section shell section-pad">
            <div class="eyebrow">03 / BUILT TO SCALE OUT</div>
            <div class="collaboration-grid"><h2>One job can use<br>more than one machine.</h2><div><p>A Voyage can send focused work to participating Vessels while keeping one trusted source of truth. Add capacity from the machines you already have without losing the thread.</p><p class="muted">Planned collaboration will let voyages share relevant progress and coordinate under permissions you control. Sessions stay private unless you choose otherwise.</p></div></div>
        </section>

        <section id="start" class="start-section shell"><div><div class="eyebrow">COMING INTO PORT</div><h2>Put your fleet<br>to work.</h2></div><div class="start-details"><p>Helm, Vessel, and Voyage are in active development. Linux comes first, followed by web and mobile access.</p><flux:button href="#surfaces" variant="primary" class="primary-button">See what’s coming <span aria-hidden="true">↗</span></flux:button><span class="release-note">A Foleybridge.Software product suite.</span></div></section>
    </main>

    <footer class="shell site-footer"><a href="/" class="wordmark">helm<span class="footer-dot">.</span>vessel<span class="footer-dot">.</span>voyage</a><p>Put every machine to work. From Foleybridge.Software.</p><a href="#main">Back to top ↑</a></footer>
</div>
