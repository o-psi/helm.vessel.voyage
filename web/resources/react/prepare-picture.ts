// Normalize camera pictures before the bounded Voyage upload. No original bytes are
// sent to Vessel or retained by the draft once conversion has succeeded.
export const MAX_PICTURE_BYTES = 4 * 1024 * 1024;
export const MAX_PICTURES = 4;
const MAX_SOURCE_BYTES = 32 * 1024 * 1024;
const MAX_SOURCE_PIXELS = 64 * 1024 * 1024;
const MAX_SIDE = 4096;
const MAX_PIXELS = 16 * 1024 * 1024;
const types = ['image/png','image/jpeg','image/webp'];

export type PreparedPicture = {blob: Blob; name: string};

export async function preparePicture(file: File, budget = MAX_PICTURE_BYTES): Promise<PreparedPicture> {
    if (budget < 16384 || budget > MAX_PICTURE_BYTES) throw new Error(`${file.name}: no image space remains in this message. Remove a picture first.`);
    const heic = /\.(heic|heif)$/i.test(file.name) || ['image/heic','image/heif'].includes(file.type);
    if (!types.includes(file.type) && !heic) throw new Error(`${file.name}: use PNG, JPEG, WebP or a browser-decodable HEIC photo.`);
    if (!file.size || file.size > MAX_SOURCE_BYTES) throw new Error(`${file.name}: select a nonempty photo under 32 MiB.`);
    // Existing compatible images retain their bytes unless they exceed either
    // transport size or the server's decoded raster dimension/pixel limits.
    if (types.includes(file.type) && file.size <= budget) {
        if (typeof createImageBitmap !== 'function') return {blob:file,name:file.name};
        try {
            const probe = await createImageBitmap(file);
            try {
                if (probe.width > 8192 || probe.height > 8192 || probe.width*probe.height > MAX_PIXELS) {
                    // Continue through bounded conversion below.
                } else return {blob:file,name:file.name};
            } finally { probe.close(); }
        } catch { return {blob:file,name:file.name}; } // Voyage validates the original signature.
    }
    if (typeof createImageBitmap !== 'function') throw new Error(`${file.name}: this browser cannot resize this photo. Export a JPEG under 4 MiB.`);
    let image: ImageBitmap;
    try { image = await createImageBitmap(file); }
    catch { throw new Error(`${file.name}: this browser cannot decode the photo. Export it as JPEG or WebP.`); }
    try {
        if (!image.width || !image.height || image.width*image.height > MAX_SOURCE_PIXELS) throw new Error(`${file.name}: photo dimensions exceed the 64 megapixel preparation limit.`);
        const scale = Math.min(1, MAX_SIDE/image.width, MAX_SIDE/image.height, Math.sqrt(MAX_PIXELS/(image.width*image.height)));
        const width = Math.max(1, Math.floor(image.width*scale)), height = Math.max(1, Math.floor(image.height*scale));
        const canvas = document.createElement('canvas'); canvas.width=width; canvas.height=height;
        const context = canvas.getContext('2d');
        if (!context) throw new Error(`${file.name}: this browser cannot prepare the photo.`);
        context.fillStyle='#fff'; context.fillRect(0,0,width,height); // transparent source becomes white in JPEG
        context.drawImage(image,0,0,width,height);
        // Downscale again if the first encoded pass is still larger than the upload budget.
        for (let pass=0; pass<5; pass++) {
            const blob = await new Promise<Blob|null>(resolve=>canvas.toBlob(resolve,'image/jpeg',pass ? 0.72 : 0.82));
            if (!blob || blob.type !== 'image/jpeg') throw new Error(`${file.name}: JPEG conversion is unavailable in this browser.`);
            if (blob.size && blob.size <= budget) {
                return {blob,name:file.name.replace(/\.[^.]+$/, '')+'.jpg'};
            }
            const nextWidth=Math.floor(canvas.width*0.7), nextHeight=Math.floor(canvas.height*0.7);
            if (!nextWidth || !nextHeight) break;
            const previous=document.createElement('canvas'); previous.width=canvas.width;previous.height=canvas.height;
            previous.getContext('2d')?.drawImage(canvas,0,0);
            canvas.width=nextWidth;canvas.height=nextHeight;
            canvas.getContext('2d')?.drawImage(previous,0,0,nextWidth,nextHeight);
        }
        throw new Error(`${file.name}: photo could not be reduced below 4 MiB.`);
    } finally { image.close(); }
}
