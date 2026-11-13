-- The same kind of script SLK_img2pixel runs, through its img2pixel table.
--
--   xd-art-cli run original_api.lua <image> <output.png>

local input, output = arg[1], arg[2]
if not input or not output then
  error("usage: original_api.lua <image> <output.png>")
end

img2pixel.size_absolute_x = 96
img2pixel.size_absolute_y = 96
img2pixel.dither_mode = img2pixel.DITHER.STUCKI
img2pixel.color_dist = img2pixel.COLORDIST.LAB_CIEDE2000
img2pixel.palette_colors = 4
img2pixel.set_palette_color(0, 0x0f380f)
img2pixel.set_palette_color(1, 0x306230)
img2pixel.set_palette_color(2, 0x8bac0f)
img2pixel.set_palette_color(3, 0x9bbc0f)

print("ok:", img2pixel.process(input, output))
