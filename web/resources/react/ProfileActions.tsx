import React,{useEffect,useId,useRef,useState} from 'react';

export function ProfileActions({name,isDefault,disabled,onEdit,onDuplicate,onDefault,onDelete}:{name:string;isDefault:boolean;disabled:boolean;onEdit:()=>void;onDuplicate:()=>void;onDefault:()=>void;onDelete:()=>void}){
    const id=useId(),trigger=useRef<HTMLButtonElement>(null),menu=useRef<HTMLDivElement>(null);
    const [open,setOpen]=useState(false),[position,setPosition]=useState({top:0,left:0});
    useEffect(()=>{
        if(!open)return;const dismiss=()=>menu.current?.hidePopover?.();
        window.addEventListener('resize',dismiss);document.addEventListener('scroll',dismiss,true);
        return()=>{window.removeEventListener('resize',dismiss);document.removeEventListener('scroll',dismiss,true);};
    },[open]);
    function positionMenu(){
        const rect=trigger.current!.getBoundingClientRect();
        // Native popovers escape the scroll container. Keep all four actions in
        // the viewport even for the last row or a narrow dialog.
        setPosition({left:Math.max(8,Math.min(rect.right-184,window.innerWidth-192)),top:Math.max(8,Math.min(rect.bottom+4,window.innerHeight-188))});
    }
    function close(){menu.current?.hidePopover?.();trigger.current?.focus();}
    function choose(action:()=>void){close();action();}
    return <>
        <button ref={trigger} type="button" className="profile-actions-trigger" aria-label={`Actions for ${name}`} title={`Actions for ${name}`} aria-haspopup="menu" aria-expanded={open} aria-controls={id} popoverTarget={id} disabled={disabled} onClick={positionMenu}>⋯</button>
        <div ref={menu} id={id} popover="auto" className="profile-actions-menu" role="menu" aria-label={`Actions for ${name}`} style={position}
            onToggle={event=>{const shown=event.newState==='open';setOpen(shown);if(shown)menu.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();}}
            onKeyDown={event=>{
                if(event.key==='Tab'){close();return;}
                if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close();return;}
                if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
                event.preventDefault();const items=[...menu.current!.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')],index=items.indexOf(document.activeElement as HTMLButtonElement);
                const next=event.key==='Home'?0:event.key==='End'?items.length-1:(index+(event.key==='ArrowDown'?1:-1)+items.length)%items.length;items[next]?.focus();
            }}>
            <button type="button" role="menuitem" tabIndex={-1} disabled={disabled} onClick={()=>choose(onEdit)}>Edit</button>
            <button type="button" role="menuitem" tabIndex={-1} disabled={disabled} onClick={()=>choose(onDuplicate)}>Duplicate</button>
            <button type="button" role="menuitem" tabIndex={-1} disabled={disabled||isDefault} onClick={()=>choose(onDefault)}>{isDefault?'Default profile':'Make default'}</button>
            <button type="button" role="menuitem" tabIndex={-1} className="setup-danger" disabled={disabled} onClick={()=>choose(onDelete)}>Delete</button>
        </div>
    </>;
}
