# ZiDevTools Image Crop

Studio can use this candidate to crop one explicitly approved static PNG, JPEG or WebP image. Enter the left and top coordinates in pixels from the upper-left corner, followed by the width and height. The rectangle must fit inside the decoded image. JPEG EXIF orientation is not applied. The output is a derived, lossless PNG, preserving transparency and the original pixels in the selected rectangle. Saving is a separate Studio confirmation and never replaces the original.

The plugin accepts one image up to 32 MiB, 4096 pixels per edge and 16,000,000 pixels total. It emits one PNG up to 32 MiB. The Studio crop capability shares the existing image Worker budget and concurrency with resize. The WASI module sees only the approved snapshot and cannot read paths or use the network. Visual selection, annotations, batch processing and automatic access to other images are outside this plugin's scope.

This unsigned candidate is built independently from the ZiDevTools source tree. The package receipt identifies the WASM and actual Windows Worker used for fixed vectors. A separate Studio acceptance records the normal picker, preview and save flow.
