#include <CoreFoundation/CoreFoundation.h>
#include <CoreGraphics/CoreGraphics.h>
#include <ImageIO/ImageIO.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../../../native/apple_image.h"
int main(int argc,char **argv){
 if(argc!=2)return 2;
 unsigned char pixels[96*64*4];
 for(int y=0;y<64;y++)for(int x=0;x<96;x++){unsigned char *p=pixels+4*(y*96+x);p[0]=220;p[1]=100;p[2]=40;p[3]=(unsigned char[]){255,128,64,0}[x/24];}
 CGColorSpaceRef cs=CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
 CGDataProviderRef provider=CGDataProviderCreateWithData(NULL,pixels,sizeof(pixels),NULL);
 CGImageRef image=CGImageCreate(96,64,8,32,96*4,cs,kCGImageAlphaLast|kCGBitmapByteOrder32Big,provider,NULL,0,kCGRenderingIntentDefault);
 CFURLRef url=CFURLCreateFromFileSystemRepresentation(NULL,(unsigned char*)argv[1],strlen(argv[1]),0);
 CGImageDestinationRef destination=CGImageDestinationCreateWithURL(url,CFSTR("public.heic"),1,NULL);
 if(!destination)return 3;
 CGImageDestinationAddImage(destination,image,NULL);
 if(!CGImageDestinationFinalize(destination))return 4;
 CFRelease(destination);CFRelease(url);CGImageRelease(image);CGDataProviderRelease(provider);CGColorSpaceRelease(cs);
 unsigned int w=0,h=0,sw=0,sh=0;unsigned char *output=photosort_apple_decode((unsigned char*)argv[1],strlen(argv[1]),0,&w,&h,&sw,&sh);
 if(!output)return 5;
 printf("dimensions %u %u source %u %u\n",w,h,sw,sh);
 for(int i=0;i<4;i++){unsigned char *p=output+4*(32*w+12+24*i);printf("sample %d RGBA %u %u %u %u\n",i,p[0],p[1],p[2],p[3]);}
 photosort_apple_free(output);return 0;
}
