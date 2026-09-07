<x-layouts.public
    page-title="Vessel — Turn every machine into agent capacity"
    page-description="Operate local and remote machines for coding agents while credentials and execution policy stay on the machine doing the work."
    :share-image="false"
>
    <x-site-header />

    <main id="main">
        <section class="product-hero shell">
            <div class="product-breadcrumb"><a href="{{ route('home') }}">Product suite</a><span>/</span> Vessel</div>
            <div class="product-hero-grid">
                <div><div class="eyebrow"><span class="status-dot"></span> Vessel / Machine fleet</div><h1>Turn every machine<br>into agent capacity.</h1></div>
                <div class="product-intro"><p>Vessel makes the computers you control available for serious agent work. Add a workstation or server, keep its authority local, and see exactly what it is carrying.</p><div class="hero-actions"><flux:button href="#benefits" variant="primary" class="primary-button">Why Vessel <span aria-hidden="true">↓</span></flux:button><a href="{{ route('products.voyage') }}" class="text-link">Next: Voyage <span aria-hidden="true">↗</span></a></div></div>
            </div>
        </section>

        <div class="product-band"><div class="shell"><strong>Your hardware.</strong><span>Your credentials.</span><span>Your access rules.</span><span>More room to run.</span></div></div>

        <section id="benefits" class="shell product-section">
            <div class="section-heading"><div class="eyebrow">WHY VESSEL</div><h2>Use the fleet<br>you already own.</h2><p>Put spare compute to work without giving up control of where code, credentials, and tools live.</p></div>
            <div class="benefit-grid">
                <article><span>01</span><h3>Add machines</h3><p>Run voyages on local or remote Vessels and choose the right machine for each job.</p></article>
                <article><span>02</span><h3>Keep secrets local</h3><p>Provider credentials stay on the executing machine instead of moving through a central service.</p></article>
                <article><span>03</span><h3>Control access</h3><p>Authorize who can observe, steer, or run work while the executing machine enforces its own policy.</p></article>
            </div>
        </section>

        <section class="product-proof shell">
            <div><div class="eyebrow">BUILT TO SUPERVISE</div><h2>Every job gets its own process.</h2><p>Vessel starts, observes, and reconnects you to independent Voyage runtimes instead of packing every agent loop into one fragile service.</p></div>
            <div><div class="eyebrow">BUILT FOR REAL NETWORKS</div><h2>Local when you’re local. Remote when you’re not.</h2><p>Helm reaches Vessels through authorized local and remote routes while the work stays on its chosen host.</p></div>
        </section>

        <section class="next-product shell"><span>Give every job a life of its own.</span><a href="{{ route('products.voyage') }}">Meet Voyage <span aria-hidden="true">↗</span></a></section>
    </main>

    <x-site-footer />
</x-layouts.public>
