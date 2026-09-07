<x-layouts.public
    page-title="Voyage — Agent work that stays alive"
    page-description="Give every coding-agent job a durable session with its own conversation, tools, progress, and execution process."
    :share-image="false"
>
    <x-site-header />

    <main id="main">
        <section class="product-hero shell">
            <div class="product-breadcrumb"><a href="{{ route('home') }}">Product suite</a><span>/</span> Voyage</div>
            <div class="product-hero-grid">
                <div><div class="eyebrow"><span class="status-dot"></span> Voyage / Persistent work</div><h1>Work that stays alive<br>after you leave.</h1></div>
                <div class="product-intro"><p>A Voyage is the work itself: one durable agent session with its own conversation, tools, progress, and process. Leave it running and return when you are ready.</p><div class="hero-actions"><flux:button href="#benefits" variant="primary" class="primary-button">Why Voyage <span aria-hidden="true">↓</span></flux:button><a href="{{ route('products.helm') }}" class="text-link">Back to Helm <span aria-hidden="true">↗</span></a></div></div>
            </div>
        </section>

        <div class="product-band"><div class="shell"><strong>One job.</strong><span>One trusted history.</span><span>Its own runtime.</span><span>Ready to continue.</span></div></div>

        <section id="benefits" class="shell product-section">
            <div class="section-heading"><div class="eyebrow">WHY VOYAGE</div><h2>Keep the context.<br>Keep the momentum.</h2><p>Stop rebuilding the story every time you reconnect, change devices, or bring another machine into the job.</p></div>
            <div class="benefit-grid">
                <article><span>01</span><h3>Stay durable</h3><p>Conversation, progress, decisions, and cleanup obligations survive interface disconnects.</p></article>
                <article><span>02</span><h3>Reach across machines</h3><p>A Voyage can assign scoped work to participating Vessels while keeping one canonical owner.</p></article>
                <article><span>03</span><h3>Know what happened</h3><p>Persistent history and exact command handling make reconnects and retries reviewable.</p></article>
            </div>
        </section>

        <section class="product-proof shell">
            <div><div class="eyebrow">INDEPENDENT BY DESIGN</div><h2>One Voyage. One runtime.</h2><p>Each session runs in its own process, so one failed job does not take every other Voyage down with it.</p></div>
            <div><div class="eyebrow">ON THE HORIZON / PLANNED</div><h2>Independent Voyages. Shared awareness.</h2><p>Planned collaboration will let Voyages share relevant progress and coordinate under permissions you control.</p></div>
        </section>

        <section class="next-product shell"><span>See every Voyage from one place.</span><a href="{{ route('products.helm') }}">Meet Helm <span aria-hidden="true">↗</span></a></section>
    </main>

    <x-site-footer />
</x-layouts.public>
