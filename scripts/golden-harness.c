/*
Golden image generator for xd-art.

Builds against the algorithm sources of SLK_img2pixel (blur.c, sample.c, sharp.c,
hscb.c, gamma.c, tint.c, dither.c, kmeans.c, shared/color.c, shared/rand.c) and writes
the results xd-art's tests compare against. See scripts/gen-goldens.sh.

Inputs are synthetic and built here, so the original's file loaders are not needed.
Output format: "XDG1", width and height as little endian u32, then RGBA bytes.
*/

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "shared/image.h"
#include "shared/color.h"
#include "img2pixel.h"

/* The original keeps these in shared/image.c, together with file format code this
   program does not need. Same behavior. */
Image8 *image8_new(int32_t w, int32_t h)
{
   Image8 *img = calloc(1, sizeof(*img) + (size_t)w * h);
   img->width = w;
   img->height = h;
   img->color_count = 256;
   return img;
}

Image32 *image32_new(int32_t w, int32_t h)
{
   Image32 *img = calloc(1, sizeof(*img) + sizeof(uint32_t) * (size_t)w * h);
   img->width = w;
   img->height = h;
   return img;
}

Image64 *image64_new(int32_t w, int32_t h)
{
   Image64 *img = calloc(1, sizeof(*img) + sizeof(uint64_t) * (size_t)w * h);
   img->width = w;
   img->height = h;
   return img;
}

Image64 *image64_dup(const Image64 *src)
{
   Image64 *img = image64_new(src->width, src->height);
   memcpy(img->data, src->data, sizeof(uint64_t) * (size_t)src->width * src->height);
   return img;
}

Image32 *image32_dup(const Image32 *src)
{
   Image32 *img = image32_new(src->width, src->height);
   memcpy(img->data, src->data, sizeof(uint32_t) * (size_t)src->width * src->height);
   return img;
}

Image64 *image32to64(const Image32 *src)
{
   Image64 *img = image64_new(src->width, src->height);
   for(int i = 0; i < src->width * src->height; i++)
      img->data[i] = color32_to_64(src->data[i]);
   return img;
}

Image32 *image64to32(const Image64 *src)
{
   Image32 *img = image32_new(src->width, src->height);
   for(int i = 0; i < src->width * src->height; i++)
      img->data[i] = color64_to_32(src->data[i]);
   return img;
}

static const char *out_dir = ".";

static void write32(const char *name, const Image32 *img)
{
   char path[1024];
   snprintf(path, sizeof(path), "%s/%s.rgba", out_dir, name);
   FILE *f = fopen(path, "wb");
   if(f == NULL)
   {
      fprintf(stderr, "could not write %s\n", path);
      exit(1);
   }
   uint8_t head[12] = {0x58, 0x44, 0x47, 0x31};
   for(int i = 0; i < 4; i++)
   {
      head[4 + i] = (uint8_t)((uint32_t)img->width >> (8 * i));
      head[8 + i] = (uint8_t)((uint32_t)img->height >> (8 * i));
   }
   fwrite(head, 1, 12, f);
   for(int i = 0; i < img->width * img->height; i++)
   {
      uint32_t c = img->data[i];
      uint8_t p[4] = {(uint8_t)c, (uint8_t)(c >> 8), (uint8_t)(c >> 16), (uint8_t)(c >> 24)};
      fwrite(p, 1, 4, f);
   }
   fclose(f);
}

static void write64(const char *name, const Image64 *img)
{
   Image32 *i32 = image64to32(img);
   write32(name, i32);
   free(i32);
}

/* Same generator as crates/xd-art-core/tests/common/mod.rs. */
static uint32_t xorshift(uint32_t *s)
{
   *s ^= *s << 13;
   *s ^= *s >> 17;
   *s ^= *s << 5;
   return *s;
}

static Image32 *fixture_gradient(void)
{
   Image32 *img = image32_new(48, 40);
   for(int y = 0; y < 40; y++)
      for(int x = 0; x < 48; x++)
         img->data[y * 48 + x] = color32((uint8_t)(x * 255 / 47), (uint8_t)(y * 255 / 39), (uint8_t)((x + y) * 7), 255);
   return img;
}

static Image32 *fixture_noise(void)
{
   Image32 *img = image32_new(48, 40);
   uint32_t s = 1;
   for(int y = 0; y < 40; y++)
      for(int x = 0; x < 48; x++)
      {
         uint32_t v = xorshift(&s);
         uint8_t a = x < 8 ? (uint8_t)(y * 6) : 255;
         img->data[y * 48 + x] = color32((uint8_t)v, (uint8_t)(v >> 8), (uint8_t)(v >> 16), a);
      }
   return img;
}

static const uint32_t pico8[16] = {
   0x000000, 0x1d2b53, 0x7e2553, 0x008751, 0xab5236, 0x5f574f, 0xc2c3c7, 0xfff1e8,
   0xff004d, 0xffa300, 0xffec27, 0x00e436, 0x29adff, 0x83769c, 0xff77a8, 0xffccaa,
};

static void dither_case(const char *name, const Image64 *src, int mode, int dist, float amount)
{
   SLK_dither_config cfg;
   memset(&cfg, 0, sizeof(cfg));
   cfg.color_dist = (SLK_color_distance)dist;
   cfg.dither_mode = (SLK_dither)mode;
   cfg.alpha_threshold = 128;
   cfg.palette_colors = 16;
   cfg.dither_amount = amount;
   cfg.target_colors = 8;
   for(int i = 0; i < 16; i++)
   {
      uint32_t c = pico8[i];
      cfg.palette[i] = color32((uint8_t)(c >> 16), (uint8_t)(c >> 8), (uint8_t)c, 255);
   }
   Image64 *work = image64_dup(src);
   SLK_img8and32 out = image64_dither(work, &cfg);
   write32(name, out.img32);
   free(out.img8);
   free(out.img32);
   free(work);
}

int main(int argc, char **argv)
{
   if(argc > 1)
      out_dir = argv[1];

   Image32 *grad32 = fixture_gradient();
   Image32 *noise32 = fixture_noise();
   write32("in_gradient", grad32);
   write32("in_noise", noise32);
   Image64 *grad = image32to64(grad32);
   Image64 *noise = image32to64(noise32);

   static const char *modes[5] = {"nearest", "bilinear", "bicubic", "lanczos", "cluster"};
   char name[256];
   for(int m = 0; m < 5; m++)
   {
      Image64 *s = image64_sample(noise, 17, 13, m, 0.0f, 0.0f);
      snprintf(name, sizeof(name), "sample_%s_noise", modes[m]);
      write64(name, s);
      free(s);
      s = image64_sample(grad, 70, 50, m, 0.3f, -0.2f);
      snprintf(name, sizeof(name), "sample_%s_up_offset", modes[m]);
      write64(name, s);
      free(s);
   }

   static const float radii[3] = {0.5f, 1.5f, 4.25f};
   static const char *radius_names[3] = {"0_5", "1_5", "4_25"};
   for(int i = 0; i < 3; i++)
   {
      Image64 *b = image64_dup(noise);
      image64_blur(b, radii[i]);
      snprintf(name, sizeof(name), "blur_%s", radius_names[i]);
      write64(name, b);
      free(b);
   }

   Image64 *small = image64_sample(noise, 24, 20, 0, 0.0f, 0.0f);
   Image64 *t = image64_dup(small);
   image64_sharpen(t, 0.8f);
   write64("adjust_sharpen", t);
   free(t);
   t = image64_dup(small);
   image64_hscb(t, 40.0f, 1.3f, 1.2f, 0.05f);
   write64("adjust_hscb", t);
   free(t);
   t = image64_dup(small);
   image64_hscb(t, 0.0f, 0.4f, 0.7f, -0.1f);
   write64("adjust_hscb_nohue", t);
   free(t);
   t = image64_dup(small);
   image64_gamma(t, 1.6f);
   write64("adjust_gamma", t);
   free(t);
   t = image64_dup(small);
   image64_tint(t, 200, 150, 255);
   write64("adjust_tint", t);
   free(t);
   free(small);

   /* Every dither mode except median cut, which depends on qsort order. */
   static const char *dithers[17] = {
      "none", "bayer8x8", "bayer4x4", "bayer2x2", "cluster8x8", "cluster4x4", "floyd",
      "floyd-mono", "median-cut", "bayer5x5", "bayer3x3", "stucki", "burkes", "sierra",
      "sierra-two-row", "sierra-lite", "picocad",
   };
   Image64 *dsrc = image64_sample(grad, 32, 24, 1, 0.0f, 0.0f);
   for(int i = 0; i < 17; i++)
   {
      if(i == 8)
         continue;
      snprintf(name, sizeof(name), "dither_%s", dithers[i]);
      dither_case(name, dsrc, i, 2, 0.4f);
   }
   static const char *metrics[6] = {"rgb", "weighted", "redmean", "cie76", "cie94", "ciede2000"};
   for(int i = 0; i < 6; i++)
   {
      snprintf(name, sizeof(name), "metric_%s", metrics[i]);
      dither_case(name, dsrc, 0, i, 0.4f);
   }
   Image64 *nsrc = image64_sample(noise, 32, 24, 0, 0.0f, 0.0f);
   dither_case("dither_floyd_alpha", nsrc, 6, 2, 0.4f);
   dither_case("dither_bayer4x4_alpha", nsrc, 2, 0, 0.9f);
   free(nsrc);
   free(dsrc);

   free(grad);
   free(noise);
   free(grad32);
   free(noise32);
   return 0;
}
