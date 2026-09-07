<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="theme-color" content="#182521">
    <title>Helm — Your agents. Your machines. Your heading.</title>
    <meta name="description" content="Meet Helm, the interface to Vessel-supervised Voyage runtimes. Independent agents, local and remote machines, and work that continues when you disconnect.">
    <link rel="canonical" href="{{ config('app.url') }}">
    <meta property="og:type" content="website">
    <meta property="og:title" content="Helm — Take the helm.">
    <meta property="og:description" content="Your agents. Your machines. Your heading. Meet Helm, Vessel and Voyage.">
    <meta property="og:url" content="{{ config('app.url') }}">
    <meta name="twitter:card" content="summary">
    @vite(['resources/css/app.css', 'resources/js/app.js'])
    @livewireStyles
    @fluxAppearance
</head>
<body>
    <a href="#main" class="skip-link">Skip to content</a>
    {{ $slot }}
    @fluxScripts
</body>
</html>
