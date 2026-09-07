@props([
    'pageTitle' => 'Helm — Put every machine to work',
    'pageDescription' => 'Run coding agents across the machines you control. Keep work moving when you disconnect, without artificial connection caps.',
    'shareImage' => true,
])
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="theme-color" content="#182521">
    <title>{{ $pageTitle }}</title>
    <meta name="description" content="{{ $pageDescription }}">
    <link rel="canonical" href="{{ url()->current() }}">
    <meta property="og:type" content="website">
    <meta property="og:title" content="{{ $pageTitle }}">
    <meta property="og:description" content="{{ $pageDescription }}">
    <meta property="og:url" content="{{ url()->current() }}">
    @if ($shareImage)
    <meta property="og:image" content="{{ rtrim(config('app.url'), '/') }}/og.png">
    <meta property="og:image:width" content="1200">
    <meta property="og:image:height" content="630">
    <meta name="twitter:card" content="summary_large_image">
    <meta name="twitter:image" content="{{ rtrim(config('app.url'), '/') }}/og.png">
    @else
    <meta name="twitter:card" content="summary">
    @endif
    <meta name="twitter:title" content="{{ $pageTitle }}">
    <meta name="twitter:description" content="{{ $pageDescription }}">
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
