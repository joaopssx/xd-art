/*
 * The three image functions SLK_img2pix_cmd takes from the original's GUI library,
 * so the command line tool builds without SDL. They do what HLH_gui_core.c does:
 * stb_image to read, stb_image_write to write. Used by scripts/compare-original.sh.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* Static copies, so they do not clash with the stb_image built into gif_read.c. */
#define STB_IMAGE_IMPLEMENTATION
#define STB_IMAGE_STATIC
#include "stb_image.h"
#define STB_IMAGE_WRITE_IMPLEMENTATION
#define STB_IMAGE_WRITE_STATIC
#include "stb_image_write.h"

uint32_t *HLH_gui_image_load(FILE *fp, int *width, int *height)
{
   int n;
   if(fp==NULL)
      return NULL;
   return (uint32_t *)stbi_load_from_file(fp, width, height, &n, 4);
}

void HLH_gui_image_free(uint32_t *pix)
{
   if(pix!=NULL)
      stbi_image_free(pix);
}

static void write_func(void *context, void *data, int size)
{
   fwrite(data, size, 1, (FILE *)context);
}

void HLH_gui_image_save(FILE *fp, uint32_t *data, int width, int height, const char *ext)
{
   if(fp==NULL||data==NULL||width<=0||height<=0)
      return;
   if(strcmp(ext,"bmp")==0)
      stbi_write_bmp_to_func(write_func, fp, width, height, 4, data);
   else if(strcmp(ext,"tga")==0)
      stbi_write_tga_to_func(write_func, fp, width, height, 4, data);
   else
      stbi_write_png_to_func(write_func, fp, width, height, 4, data, width * 4);
}
