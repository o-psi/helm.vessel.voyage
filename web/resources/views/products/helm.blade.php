<x-layouts.public
    page-title="Helm — One command center for every agent"
    page-description="See, steer, and return to coding-agent work across local and remote machines from one focused command center."
    :share-image="false"
>
    <x-site-header />

    <main id="main">
        <section class="product-hero shell">
            <div class="product-breadcrumb"><a href="{{ route('home') }}">Product suite</a><span>/</span> Helm</div>
            <div class="product-hero-grid">
                <div><div class="eyebrow"><span class="status-dot"></span> Helm / Command center</div><h1>Stay in control<br>without standing still.</h1></div>
                <div class="product-intro"><p>Helm gives you one place to start work, watch it move, and step in when an agent needs you. Close the interface and the job keeps going.</p><div class="hero-actions"><flux:button href="#benefits" variant="primary" class="primary-button">Why Helm <span aria-hidden="true">↓</span></flux:button><a href="{{ route('products.vessel') }}" class="text-link">Next: Vessel <span aria-hidden="true">↗</span></a></div></div>
            </div>
        </section>

        <div class="product-band"><div class="shell"><strong>One view.</strong><span>Every voyage.</span><span>Every machine.</span><span>No artificial connection cap.</span></div></div>

        <section id="benefits" class="shell product-section">
            <div class="section-heading"><div class="eyebrow">WHY HELM</div><h2>See the work.<br>Steer the work.</h2><p>Give every agent room to run without losing the ability to jump in at the right moment.</p></div>
            <div class="benefit-grid">
                <article><span>01</span><h3>Run more at once</h3><p>Move between independent voyages instead of waiting for one terminal session to finish.</p></article>
                <article><span>02</span><h3>Come back anytime</h3><p>Disconnect without cancelling accepted work. Reopen Helm and return to the same session.</p></article>
                <article><span>03</span><h3>Stay close to decisions</h3><p>See progress, answer approvals, and redirect work from one focused interface.</p></article>
            </div>
        </section>

        <section class="product-proof shell">
            <div><div class="eyebrow">AVAILABLE FIRST</div><h2>Fast by keyboard.</h2><p>The Linux CLI and TUI are the first Helm experience, built for people who live in the terminal.</p></div>
            <div><div class="eyebrow">COMING NEXT / PLANNED</div><h2>Ready when you leave it.</h2><p>Planned web and mobile interfaces will let you check progress and steer the same voyages away from your desk.</p></div>
        </section>

        <section class="next-product shell"><span>Give Helm more machines to command.</span><a href="{{ route('products.vessel') }}">Meet Vessel <span aria-hidden="true">↗</span></a></section>
    </main>

    <x-site-footer />
</x-layouts.public>
