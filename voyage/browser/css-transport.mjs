// Source-created, lossless CSS dictionary codec shared by worker and recorder.
// This factory is injected before website scripts; it opens no page-side channel.
export function createCssCodec(){
 const LIMIT=8*1024*1024,MAX_ITEMS=20000,MAX_VISITS=200000,MAX_DEPTH=256;
 const encoder=new TextEncoder(), cssKeys=new Set(['_cssText','style','cssText','rule','replace','replaceSync']);
 const fail=()=>{throw Error('css_transport_limit');};
 const stringBytes=value=>{if(value.length>LIMIT)fail();return encoder.encode(JSON.stringify(value)).length;};
 function jsonBytes(value){
  let bytes=0,items=0,nodes=0;const ancestors=new Set();
  const add=count=>{bytes+=count;if(bytes>LIMIT)fail();};
  const visit=(item,depth)=>{
   if(depth>MAX_DEPTH)fail();
   if(item===null||typeof item==='boolean'||typeof item==='number'){add(JSON.stringify(item).length);return;}
   if(typeof item==='string'){add(stringBytes(item));return;}
   if(!item||typeof item!=='object'||ancestors.has(item)||++items>MAX_VISITS)fail();
   if(Number.isSafeInteger(item.id)&&Number.isSafeInteger(item.type)&&++nodes>MAX_ITEMS)fail();
   ancestors.add(item);add(2);
   if(Array.isArray(item)){for(let i=0;i<item.length;i++){if(i)add(1);visit(item[i]===undefined?null:item[i],depth+1);}}
   else {let first=true;for(const [key,child] of Object.entries(item)){if(child===undefined)continue;if(!first)add(1);first=false;add(stringBytes(key)+1);visit(child,depth+1);}}
   ancestors.delete(item);
  };
  visit(value,0);return bytes;
 }
 function pack(events){
  if(!Array.isArray(events)||events.length>1024)fail();
  const expanded_bytes=jsonBytes(events),dictionary=[],indexes=new Map();let refs=0;
  const visit=(value,key='',style=false,depth=0)=>{
   if(depth>MAX_DEPTH)fail();
   if(typeof value==='string'&&(cssKeys.has(key)||key==='textContent'&&style)){
    if(++refs>MAX_ITEMS)fail();let index=indexes.get(value);
    if(index===undefined){index=dictionary.length;indexes.set(value,index);dictionary.push(value);}
    return {$css:index};
   }
   if(!value||typeof value!=='object')return value;
   const inside=style||value.tagName==='style';
   if(Array.isArray(value))return value.map(child=>visit(child,'',inside,depth+1));
   const result={};for(const [name,child] of Object.entries(value))if(child!==undefined)Object.defineProperty(result,name,{value:visit(child,name,inside,depth+1),enumerable:true,writable:true,configurable:true});return result;
  };
  const payload={events:visit(events),css_dictionary:dictionary};jsonBytes(payload);
  return {payload,expanded_bytes};
 }
 function unpack(payload,maxBytes=LIMIT){
  if(!Number.isSafeInteger(maxBytes)||maxBytes<=0||maxBytes>LIMIT)fail();
  if(!payload||typeof payload!=='object'||Array.isArray(payload)||Object.keys(payload).length!==2||!Array.isArray(payload.events)||payload.events.length>1024||!Array.isArray(payload.css_dictionary)||payload.css_dictionary.length>MAX_ITEMS||payload.css_dictionary.some(value=>typeof value!=='string'))fail();
  if(jsonBytes(payload)>maxBytes)fail();const used=new Set();let refs=0;
  const visit=(value,key='',style=false,depth=0)=>{
   if(depth>MAX_DEPTH)fail();
   if(value&&typeof value==='object'&&!Array.isArray(value)&&Object.hasOwn(value,'$css')){
    if(Object.keys(value).length!==1||!(cssKeys.has(key)||key==='textContent'&&style)||!Number.isSafeInteger(value.$css)||value.$css<0||value.$css>=payload.css_dictionary.length||++refs>MAX_ITEMS)fail();
    used.add(value.$css);return payload.css_dictionary[value.$css];
   }
   if(!value||typeof value!=='object')return value;
   const inside=style||value.tagName==='style';
   if(Array.isArray(value))return value.map(child=>visit(child,'',inside,depth+1));
   const result={};for(const [name,child] of Object.entries(value))Object.defineProperty(result,name,{value:visit(child,name,inside,depth+1),enumerable:true,writable:true,configurable:true});return result;
  };
  // Check expanded lengths before allocating the restored object graph.
  const charge=(value,key='',style=false,depth=0)=>{
   if(depth>MAX_DEPTH)fail();
   if(value&&typeof value==='object'&&!Array.isArray(value)&&Object.hasOwn(value,'$css')){
    if(Object.keys(value).length!==1||!(cssKeys.has(key)||key==='textContent'&&style)||!Number.isSafeInteger(value.$css)||value.$css<0||value.$css>=payload.css_dictionary.length)fail();return stringBytes(payload.css_dictionary[value.$css]);
   }
   if(value===null||typeof value==='boolean'||typeof value==='number')return JSON.stringify(value).length;
   if(typeof value==='string')return stringBytes(value);
   if(!value||typeof value!=='object')fail();const inside=style||value.tagName==='style';let bytes=2;
   if(Array.isArray(value)){for(let i=0;i<value.length;i++){bytes+=(i?1:0)+charge(value[i],'',inside,depth+1);if(bytes>LIMIT)fail();}}
   else {let first=true;for(const [name,child] of Object.entries(value)){bytes+=(first?0:1)+stringBytes(name)+1+charge(child,name,inside,depth+1);first=false;if(bytes>LIMIT)fail();}}
   return bytes;
  };
  const expanded_bytes=charge(payload.events);if(expanded_bytes>maxBytes)fail();
  const events=visit(payload.events);if(used.size!==payload.css_dictionary.length)fail();
  return {events,expanded_bytes};
 }
 return Object.freeze({pack,unpack,jsonBytes,limit:LIMIT,chunkBytes:512*1024,maxChunks:16});
}
