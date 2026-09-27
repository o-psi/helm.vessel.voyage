import test from 'node:test';
import assert from 'node:assert/strict';
import {preparePicture, MAX_PICTURE_BYTES} from '../resources/react/prepare-picture.ts';

test('large camera JPEG is resized locally into bounded upload bytes', async () => {
    const oldBitmap=globalThis.createImageBitmap, oldDocument=globalThis.document;
    let closed=false, passes=0;
    const canvas:any={width:0,height:0,getContext:()=>({fillRect(){},drawImage(){},set fillStyle(_value:string){}}),toBlob:(callback:(blob:Blob)=>void)=>callback(new Blob([new Uint8Array(++passes===1 ? MAX_PICTURE_BYTES+1 : 4096)],{type:'image/jpeg'}))};
    Object.assign(globalThis,{createImageBitmap:async()=>({width:6000,height:4000,close(){closed=true;}}),document:{createElement:()=>({...canvas})}});
    try {
        const file=new File([new Uint8Array(5*1024*1024)],'camera.jpg',{type:'image/jpeg'});
        const prepared=await preparePicture(file);
        assert.equal(prepared.name,'camera.jpg');assert.equal(prepared.blob.size,4096);
        assert.equal(prepared.blob.type,'image/jpeg');assert.equal(passes,2);assert.equal(closed,true);
    } finally {Object.assign(globalThis,{createImageBitmap:oldBitmap,document:oldDocument});}
});

test('HEIC needs browser decode, unknown types and huge sources refuse before decoding',async()=>{
    const original=globalThis.createImageBitmap;
    let calls=0;
    globalThis.createImageBitmap=(async()=>{calls++;throw new Error('unsupported');}) as any;
    try {
        await assert.rejects(preparePicture(new File([1,2],'camera.heic',{type:'image/heic'})),/browser cannot decode/);
        await assert.rejects(preparePicture(new File([1,2],'vector.svg',{type:'image/svg+xml'})),/use PNG, JPEG, WebP/);
        await assert.rejects(preparePicture(new File([new Uint8Array(33*1024*1024)],'huge.jpg',{type:'image/jpeg'})),/under 32 MiB/);
        assert.equal(calls,1);
    } finally {globalThis.createImageBitmap=original;}
});

test('valid 3 MiB JPEG is retained without lossy conversion',async()=>{
    const file=new File([new Uint8Array(3*1024*1024)],'camera.jpg',{type:'image/jpeg'});
    const original=globalThis.createImageBitmap;
    globalThis.createImageBitmap=(async()=>({width:2048,height:1536,close(){}})) as any;
    try {const prepared=await preparePicture(file);assert.equal(prepared.blob,file);assert.equal(prepared.name,'camera.jpg');}
    finally {globalThis.createImageBitmap=original;}
});
