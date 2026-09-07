<div>
    <x-site-header />

    <main id="main">
        <section class="hero shell compact-hero">
            <div class="hero-copy">
                <div class="eyebrow"><span class="status-dot"></span> Your machines are ready for more</div>
                <h1>Put every<br>machine<br><em>to work.</em></h1>
                <p class="hero-description">Run coding agents across the computers you already own. Start more work, keep it moving when you disconnect, and steer it all from Helm.</p>
                <div class="hero-actions">
                    <flux:button href="{{ route('products.helm') }}" variant="primary" class="primary-button">Meet Helm <span aria-hidden="true">↗</span></flux:button>
                    <a href="#products" class="text-link">Explore the suite <span aria-hidden="true">↓</span></a>
                </div>
                <p class="release-note"><span class="status-dot"></span> Built for Linux first <span class="separator">/</span> Web and mobile are next</p>
            </div>

            <div class="system-map" aria-label="Helm connects to local and remote Vessels running independent Voyages">
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

        <section id="products" class="system-section shell landing-products">
            <div class="section-heading"><div class="eyebrow">THE PRODUCT SUITE</div><h2>Three products.<br>One job: ship more.</h2><p>Start with the result you want, then see what each product brings to the crew.</p></div>
            <div class="component-grid linked-products">
                <a class="component-card" href="{{ route('products.helm') }}"><div class="card-index">01 <span>COMMAND CENTER</span></div><h3>Helm<span>.</span></h3><p>See every session and steer work across your machines.</p><div class="card-footer">Meet Helm <span aria-hidden="true">↗</span></div></a>
                <a class="component-card" href="{{ route('products.vessel') }}"><div class="card-index">02 <span>MACHINE FLEET</span></div><h3>Vessel<span>.</span></h3><p>Turn the computers you control into dependable agent capacity.</p><div class="card-footer">Meet Vessel <span aria-hidden="true">↗</span></div></a>
                <a class="component-card" href="{{ route('products.voyage') }}"><div class="card-index">03 <span>PERSISTENT WORK</span></div><h3>Voyage<span>.</span></h3><p>Keep every agent session alive, durable, and ready to continue.</p><div class="card-footer">Meet Voyage <span aria-hidden="true">↗</span></div></a>
            </div>
        </section>

        <section class="start-section shell compact-start"><div><div class="eyebrow">COMING INTO PORT</div><h2>Your fleet is waiting.</h2></div><div class="start-details"><p>Helm, Vessel, and Voyage are in active development from Foleybridge.Software.</p><flux:button href="{{ route('products.helm') }}" variant="primary" class="primary-button">See what ships first <span aria-hidden="true">↗</span></flux:button></div></section>
    </main>

    <x-site-footer />
</div>
