---
license: agpl-3.0
---

# Info

Since people are downloading this and I don't know why, I'll add some information. This model is an image classifier fine-tuned on `microsoft/beit-base-patch16-384`.
Its purpose is to be used in the dataset conditioning step for the [Waifu Diffusion project](https://huggingface.co/hakurei/waifu-diffusion), a fine-tune effort for Stable Diffusion. As WD1.4 is planned to have a *significantly large dataset* (~15m images), it is infeasible to analyze every image manually to determine whether or not it should be included in the final training dataset. This image classifier is trained on approximately 3.5k real-life and anime/manga images. Its purpose is to remove aesthetically worthless images from our dataset by classifying them as "`not_aesthetic`". The image classifier was trained to **err on the side of caution** and will generally tend to include images unless they are in a "manga-like" format, have messy lines and/or are sketches, or include an unacceptable amount of text (namely text that covers the primary subject of the image). The idea is that certain images will hurt a SD fine-tune.

Note: This classifier is not perfect, just like every other classifier out there. However, with a sufficiently large dataset, any imperfections or misclassifications should average themselves out due to the Law of Large Numbers.

You can test out the classifier [here](https://huggingface.co/spaces/cafeai/cafe_aesthetic_demo), along with some other classifiers for the project.


# License
Released under the aGPLv3. Use the model as you wish for any purpose. If you make changes, share the changes.