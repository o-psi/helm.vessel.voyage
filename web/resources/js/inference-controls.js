// Provider strings stay canonical; slider positions are only presentation indices.
export function setReasoning(field, items, selected = '') {
    field.reasoningItems = items;
    field.setAttribute('max', String(Math.max(1, items.length - 1)));
    field.value = String(Math.max(0, items.findIndex(item => item.value === selected)));
    field.disabled = items.length < 2;
    const update = () => {
        const item = items[Number(field.value)] || items[0];
        const label = document.getElementById(`${field.id}-value`);
        if (label) label.textContent = item?.label || 'Provider default';
        for (const input of field.querySelectorAll('input')) input.setAttribute('aria-valuetext', item?.label || 'Provider default');
    };
    field.oninput = update; field.onchange = update; update();
}
export function reasoningValue(field) {
    return field.reasoningItems?.[Number(field.value)]?.value || '';
}
