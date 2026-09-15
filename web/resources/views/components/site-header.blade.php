<header class="site-header shell">
    <a class="wordmark" href="{{ route('home') }}" aria-label="Helm, Vessel, and Voyage home"><span class="brand-mark" aria-hidden="true">h.</span> helm</a>
    <nav aria-label="Main navigation">
        <a href="{{ route('products.helm') }}">Helm</a>
        <a href="{{ route('products.vessel') }}">Vessel</a>
        <a href="{{ route('products.voyage') }}">Voyage</a>
        @if (\App\Services\ConsoleAccess::enabled())
            <a href="{{ \App\Services\ConsoleAccess::authenticated(request()) ? route('console') : route('console.login') }}" class="nav-cta">{{ \App\Services\ConsoleAccess::authenticated(request()) ? 'Open console' : 'Sign in' }} <span aria-hidden="true">↗</span></a>
        @else
            <a href="{{ route('products.helm') }}" class="nav-cta">Explore the suite <span aria-hidden="true">↗</span></a>
        @endif
    </nav>
</header>
