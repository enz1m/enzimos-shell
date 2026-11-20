#!/usr/bin/env python3
from pathlib import Path
import argparse
import math
from PIL import Image, ImageDraw, ImageFilter

SIZE = (2560, 1440)


def gradient(top, bottom):
    image = Image.new('RGB', SIZE)
    draw = ImageDraw.Draw(image)
    for y in range(SIZE[1]):
        t = y / (SIZE[1] - 1)
        draw.line((0, y, SIZE[0], y), fill=tuple(round(a + (b-a)*t) for a, b in zip(top, bottom)))
    return image


def glow(image, box, color, blur):
    layer = Image.new('RGBA', SIZE)
    ImageDraw.Draw(layer).ellipse(box, fill=color)
    layer = layer.filter(ImageFilter.GaussianBlur(blur))
    image.paste(layer, (0, 0), layer)


def ridge(image, base, amplitude, phase, color, waves=1.0):
    points = [(x, base + int(amplitude*math.sin(x/2560*math.pi*waves+phase))) for x in range(-10, 2571, 10)]
    ImageDraw.Draw(image).polygon(points+[(2560,1440),(0,1440)], fill=color)


def pictures():
    image = gradient((14, 23, 56), (36, 65, 87))
    glow(image, (850, -460, 2650, 720), (58, 193, 185, 155), 150)
    layer = Image.new('RGBA', SIZE)
    draw = ImageDraw.Draw(layer)
    for i in range(34):
        points = [(x, int(420 + i*9 + 180*math.sin(x/780+i*.008))) for x in range(-20,2580,8)]
        draw.line(points, fill=(77, 229-i*2, 197, 60), width=13)
    image = Image.alpha_composite(image.convert('RGBA'), layer.filter(ImageFilter.GaussianBlur(18))).convert('RGB')
    ridge(image, 1190, 90, 1.8, (15, 32, 52), 2.2)
    yield 'enzui-aurora', image

    image = gradient((111, 103, 151), (255, 194, 149))
    glow(image, (1610, 360, 2020, 770), (255, 234, 180, 230), 40)
    ImageDraw.Draw(image).ellipse((1680,430,1950,700), fill=(255,227,173))
    for base, amp, phase, color in [(915,120,.8,(192,137,141)),(1090,170,2.0,(156,98,118)),(1290,210,3.8,(107,63,96))]:
        ridge(image,base,amp,phase,color,1.7)
    yield 'enzui-dusk-dunes', image

    image = gradient((66, 107, 139), (211, 213, 193))
    glow(image,(1250,80,1810,630),(246,212,166,170),100)
    draw = ImageDraw.Draw(image)
    draw.polygon([(0,1160),(490,790),(780,890),(1150,460),(1490,865),(1770,665),(2560,1220),(2560,1440),(0,1440)],fill=(107,136,153))
    draw.polygon([(923,720),(1150,460),(1330,676),(1220,649),(1160,563),(1088,688),(1030,639)],fill=(216,226,224))
    draw.polygon([(0,1260),(600,940),(990,1310),(1710,800),(2200,1100),(2560,905),(2560,1440),(0,1440)],fill=(62,101,122))
    ridge(image,1330,85,2.0,(30,70,94),2.0)
    yield 'enzui-alpine-dawn',image

    image = gradient((132, 182, 186), (23, 86, 107))
    for n in range(7):
        ridge(image,570+n*145,100+n*10,n*.62,tuple(round(a+(b-a)*n/6) for a,b in zip((139,189,189),(13,54,81))),2.2)
    yield 'enzui-tidal',image

    image=gradient((56,37,75),(186,109,105))
    glow(image,(185,80,1265,1030),(228,141,104,110),160)
    ImageDraw.Draw(image).ellipse((550,345,950,745),fill=(245,181,123))
    ridge(image,1020,180,.0,(140,72,91),1.6)
    ridge(image,1210,200,2.2,(97,48,79),1.8)
    ridge(image,1380,160,4.5,(56,33,62),2.4)
    yield 'enzui-ember',image

    image=gradient((27,30,67),(65,46,100))
    glow(image,(1510,-160,2900,1290),(114,82,173,160),130)
    glow(image,(-900,200,770,2010),(33,118,156,165),150)
    layer=Image.new('RGBA',SIZE)
    draw=ImageDraw.Draw(layer)
    for n in range(20):
        draw.arc((500+n*7,-620+n*14,2700+n*7,1580+n*14),32,210,fill=(185,164,230,40+n*3),width=3)
    image=Image.alpha_composite(image.convert('RGBA'),layer).convert('RGB')
    yield 'enzui-orbit',image


if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('directory',type=Path)
    args=parser.parse_args()
    args.directory.mkdir(parents=True,exist_ok=True)
    for name,image in pictures():
        with (args.directory/(name+'.jpg')).open('xb') as file:
            image.save(file,format='JPEG',quality=94,subsampling=0,optimize=True)
        print(name,flush=True)
