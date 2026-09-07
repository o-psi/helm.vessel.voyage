<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="theme-color" content="#182521">
    <title>Helm — Put every machine to work</title>
    <meta name="description" content="Run coding agents across the machines you control. Keep work moving when you disconnect, without artificial connection caps.">
    <link rel="canonical" href="{{ config('app.url') }}">
    <meta property="og:type" content="website">
    <meta property="og:title" content="Helm — Put every machine to work">
    <meta property="og:description" content="Run more coding agents across your machines. Keep every session moving, wherever you are.">
    <meta property="og:url" content="{{ config('app.url') }}">
    <meta property="og:image" content="{{ rtrim(config('app.url'), '/') }}/og.png">
    <meta property="og:image:width" content="1200">
    <meta property="og:image:height" content="630">
    <meta name="twitter:card" content="summary_large_image">
    <meta name="twitter:title" content="Helm — Put every machine to work">
    <meta name="twitter:description" content="Run more coding agents across your machines. Keep every session moving, wherever you are.">
    <meta name="twitter:image" content="{{ rtrim(config('app.url'), '/') }}/og.png">
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
