<div>
    <header class="site-header shell">
        <a class="wordmark" href="/" aria-label="Helm home"><span class="brand-mark" aria-hidden="true">h.</span> helm</a>
        <nav aria-label="Main navigation">
            <a href="#system">The system</a>
            <a href="#surfaces">The interfaces</a>
            <a href="#start" class="nav-cta">Get started <span aria-hidden="true">↗</span></a>
        </nav>
    </header>

    <main id="main">
        <section class="hero shell">
            <div class="hero-copy">
                <div class="eyebrow"><span class="status-dot"></span> Independent agents. Shared direction.</div>
                <h1>Take<br>the <em>helm.</em></h1>
                <p class="hero-description">Your agents. Your machines. Your heading.<br>One place to steer work across local and remote machines—while every voyage keeps its own course.</p>
                <div class="hero-actions">
                    <flux:button href="#start" variant="primary" class="primary-button">Explore Helm <span aria-hidden="true">↗</span></flux:button>
                    <a href="#system" class="text-link">Meet the three parts <span aria-hidden="true">↓</span></a>
                </div>
                <p class="release-note"><span class="status-dot"></span> Linux CLI / TUI today <span class="separator">/</span> Web & mobile on the horizon</p>
            </div>

            <div class="system-map" aria-label="Architecture illustration: Helm connects to local and remote Vessels, which supervise independent voyages">
                <div class="map-topline"><span>YOUR OPERATING PICTURE</span><span>01 — 03</span></div>
                <div class="helm-node"><span class="node-symbol">h.</span><div><strong>Helm</strong><span>You set the direction</span></div><span class="node-tag">INTERFACE</span></div>
                <div class="map-trunk" aria-hidden="true"></div>
                <div class="vessel-columns">
                    <div class="vessel-group"><div class="vessel-label"><span class="status-dot"></span> Local Vessel</div><div class="voyage-node"><span>↗</span> Voyage A <small>INDEPENDENT</small></div><div class="voyage-node"><span>↗</span> Voyage B <small>INDEPENDENT</small></div><div class="machine-caption">YOUR WORKSTATION</div></div>
                    <div class="vessel-group"><div class="vessel-label"><span class="status-dot"></span> Remote Vessel</div><div class="voyage-node"><span>↗</span> Voyage C <small>INDEPENDENT</small></div><div class="voyage-node"><span>↗</span> Voyage D <small>INDEPENDENT</small></div><div class="machine-caption">YOUR SERVER</div></div>
                </div>
                <div class="map-bottomline"><span>One owner per voyage.</span><span>Built to stay the course.</span></div>
            </div>
        </section>

        <div class="principles"><div class="shell principles-inner"><span>Built in Rust</span><span>No artificial connection cap</span><span>Local + remote execution</span><span>Disconnect without cancelling</span></div></div>

        <section id="system" class="system-section shell section-pad">
            <div class="section-heading"><div class="eyebrow">01 / THE SYSTEM</div><h2>Three parts.<br>One direction.</h2><p>The name is the architecture.<br>Each part has a job. All three matter.</p></div>
            <div class="component-grid">
                <article class="component-card"><div class="card-index">01 <span>THE INTERFACE</span></div><h3>Helm<span>.</span></h3><p>Your place to steer. Connect to local and remote Vessels, move between voyages, and stay close to the work.</p><div class="card-footer">Where you take control <span aria-hidden="true">↗</span></div></article>
                <article class="component-card"><div class="card-index">02 <span>THE SUPERVISOR</span></div><h3>Vessel<span>.</span></h3><p>The service on your machine. Starts independent voyage processes and gives Helm authorized access to their state.</p><div class="card-footer">Where work is supervised <span aria-hidden="true">↗</span></div></article>
                <article class="component-card"><div class="card-index">03 <span>THE RUNTIME</span></div><h3>Voyage<span>.</span></h3><p>An ongoing session with its own process. Owns the conversation, agent, tools and history, even when you close Helm.</p><div class="card-footer">Where execution happens <span aria-hidden="true">↗</span></div></article>
            </div>
            <div class="domain-line"><span>THE WHOLE SYSTEM, IN ONE ADDRESS</span><p><strong>helm</strong><i>.</i><strong>vessel</strong><i>.</i><strong>voyage</strong></p></div>
        </section>

        <section id="surfaces" class="surfaces-section section-pad">
            <div class="shell surface-grid">
                <div><div class="eyebrow">02 / YOUR WAY IN</div><h2>Same heading.<br>Different horizons.</h2><p class="section-description">Start at your terminal. The vision extends to your browser and your pocket, with the same independent runtimes behind each interface.</p>
                    <div class="surface-tabs" role="group" aria-label="Explore Helm interfaces">
                        @foreach (['terminal' => 'CLI / TUI', 'web' => 'Web', 'mobile' => 'Mobile'] as $key => $label)
                            <flux:button wire:click="selectSurface('{{ $key }}')" :variant="$surface === $key ? 'primary' : 'ghost'" aria-pressed="{{ $surface === $key ? 'true' : 'false' }}">{{ $label }}</flux:button>
                        @endforeach
                    </div>
                </div>
                <div class="surface-panel" aria-live="polite" wire:loading.attr="aria-busy">
                    @if ($surface === 'terminal')
                        <div class="panel-label"><span class="status-dot"></span> AVAILABLE ON LINUX</div><h3>At home in<br>your terminal.</h3><p>A keyboard-first interface for local and remote voyages. Switch sessions, inspect work and respond to requests without owning the execution process.</p><div class="terminal-line"><span>$</span> helm <span class="cursor" aria-hidden="true"></span></div>
                    @elseif ($surface === 'web')
                        <div class="panel-label">ON THE HORIZON / PLANNED</div><h3>A view from<br>your browser.</h3><p>A planned browser interface to your Vessels and voyages. This website introduces the system; the browser execution console is not available yet.</p><div class="surface-caption">YOUR BROWSER → HELM → VESSEL → VOYAGE</div>
                    @else
                        <div class="panel-label">ON THE HORIZON / PLANNED</div><h3>Stay close.<br>Go further.</h3><p>A planned mobile app for staying connected to your work away from the desk. Mobile access and approval workflows are still being developed.</p><div class="surface-caption">YOUR POCKET → HELM → VESSEL → VOYAGE</div>
                    @endif
                </div>
            </div>
        </section>

        <section class="collaboration-section shell section-pad">
            <div class="eyebrow">03 / A SHARED DIRECTION</div>
            <div class="collaboration-grid"><h2>Independent work.<br>Connected possibilities.</h2><div><p>A voyage can delegate scoped work across participating Vessels while keeping one canonical owner. Your machines contribute without creating competing copies of the session.</p><p class="muted">The broader vision: voyages discover relevant activity and coordinate with one another under explicit permissions. A shared machine never means unrestricted access to another session.</p></div></div>
        </section>

        <section id="start" class="start-section shell"><div><div class="eyebrow">SET YOUR HEADING</div><h2>Every voyage<br>starts at the helm.</h2></div><div class="start-details"><p>Helm is in active development. The Linux CLI / TUI comes first, with public downloads to follow.</p><flux:button href="#surfaces" variant="primary" class="primary-button">Explore the interfaces <span aria-hidden="true">↗</span></flux:button><span class="release-note">A first look at the system. More on the horizon.</span></div></section>
    </main>

    <footer class="shell site-footer"><a href="/" class="wordmark">helm<span class="footer-dot">.</span></a><p>Helm interfaces. Vessel supervises. Voyage executes.</p><a href="#main">Back to top ↑</a></footer>
</div>
