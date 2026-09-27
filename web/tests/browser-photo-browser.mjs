// Local Chromium fixture for camera-photo normalization, without a Voyage or provider.
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve,dirname} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import assert from 'node:assert/strict';
import {transform} from 'esbuild';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const {chromium}=await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE||`${root}/voyage/browser/node_modules/playwright-core/index.mjs`).href);
const source=await readFile(`${root}/web/resources/react/prepare-picture.ts`,'utf8');
const module=(await transform(source,{loader:'ts',format:'esm',target:'es2022'})).code;
const server=createServer((request,response)=>{
    response.setHeader('Content-Type',request.url==='/prepare-picture.js'?'text/javascript':'text/html');
    response.end(request.url==='/prepare-picture.js'?module:'<html><body>Local photo fixture</body></html>');
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
let browser;
try {
    browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH||'/usr/bin/chromium',headless:true});
    const page=await browser.newPage();await page.goto(`http://127.0.0.1:${server.address().port}/`);
    const result=await page.evaluate(async()=>{
        const {preparePicture}=await import('/prepare-picture.js');
        const canvas=document.createElement('canvas');canvas.width=3400;canvas.height=2500;
        const context=canvas.getContext('2d');const pixels=context.createImageData(canvas.width,canvas.height);
        let seed=1;for(let i=0;i<pixels.data.length;i+=4){seed=(Math.imul(seed,1664525)+1013904223)|0;pixels.data[i]=seed&255;pixels.data[i+1]=(seed>>>8)&255;pixels.data[i+2]=(seed>>>16)&255;pixels.data[i+3]=255;}
        context.putImageData(pixels,0,0);
        const source=await new Promise(resolve=>canvas.toBlob(resolve,'image/jpeg',0.98));
        const prepared=await preparePicture(new File([source],'camera.jpg',{type:'image/jpeg'}));
        const decoded=await createImageBitmap(prepared.blob);
        const result={original:source.size,compressed:prepared.blob.size,format:prepared.blob.type,width:decoded.width,height:decoded.height};
        decoded.close();return result;
    });
    assert.ok(result.original>4*1024*1024,`fixture needs a large camera photo: ${result.original}`);
    assert.ok(result.compressed>0&&result.compressed<=4*1024*1024);
    assert.equal(result.format,'image/jpeg');assert.ok(result.width*result.height<=16*1024*1024);
    console.log(JSON.stringify({browser:browser.version(),result}));
} finally {await browser?.close();await new Promise(resolve=>server.close(resolve));}
