// Based on CAS sample
//
// Copyright(c) 2019 Advanced Micro Devices, Inc.All rights reserved.
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files(the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions :
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

// @[export]
// @[internal_buffer]
layout(set=0,binding=0) uniform Config
{
    //uvec4 const0;
    //uvec4 const1;
    uint image_width;
    uint image_height;
    float sharpen_amount;
} config;

// @[export]
layout(set=0,binding=1,rgba16) uniform image2D img_src;

// @[export]
layout(set=0,binding=2,rgba16) uniform image2D img_dst;

#define A_GPU 1
#define A_GLSL 1

#if CAS_SAMPLE_FP16

#define A_HALF 1
#define CAS_PACKED_ONLY 1

#endif

#include "ffx_a.h"

#if CAS_SAMPLE_FP16

AH3 CasLoadH(ASW2 p)
{
    return AH3(imageLoad(img_src,ASU2(p)).rgb);
}

// Lets you transform input from the load into a linear color space between 0 and 1. See ffx_cas.h
// In this case, our input is already linear and between 0 and 1
void CasInputH(inout AH2 r, inout AH2 g, inout AH2 b) {}

#else

AF3 CasLoad(ASU2 p)
{
    return imageLoad(img_src,p).rgb;
}

// Lets you transform input from the load into a linear color space between 0 and 1. See ffx_cas.h
// In this case, our input is already linear and between 0 and 1
void CasInput(inout AF1 r, inout AF1 g, inout AF1 b) {}

#endif

#include "ffx_cas.h"

layout(local_size_x=64) in;
void main()
{
    uvec4 const0;
    uvec4 const1;
    CasSetup(const0, const1, config.sharpen_amount, config.image_width, config.image_width, config.image_width, config.image_width);

    // Do remapping of local xy in workgroup for a more PS-like swizzle pattern.
    AU2 gxy = ARmp8x8(gl_LocalInvocationID.x)+AU2(gl_WorkGroupID.x<<4u,gl_WorkGroupID.y<<4u);
    bool sharpenOnly = true;

#if CAS_SAMPLE_FP16

    // Filter.
    AH4 c0, c1;
    AH2 cR, cG, cB;

    CasFilterH(cR, cG, cB, gxy, const0, const1, sharpenOnly);
    CasDepack(c0, c1, cR, cG, cB);
    imageStore(img_dst, ASU2(gxy), AF4(c0));
    imageStore(img_dst, ASU2(gxy)+ASU2(8,0), AF4(c1));
    gxy.y += 8u;

    CasFilterH(cR, cG, cB, gxy, const0, const1, sharpenOnly);
    CasDepack(c0, c1, cR, cG, cB);
    imageStore(img_dst, ASU2(gxy), AF4(c0));
    imageStore(img_dst, ASU2(gxy)+ASU2(8,0), AF4(c1));

#else

    // Filter.
    AF4 c;
    CasFilter(c.r, c.g, c.b, gxy, const0, const1, sharpenOnly);
    imageStore(img_dst, ASU2(gxy), c);
    gxy.x += 8u;

    CasFilter(c.r, c.g, c.b, gxy, const0, const1, sharpenOnly);
    imageStore(img_dst, ASU2(gxy), c);
    gxy.y += 8u;

    CasFilter(c.r, c.g, c.b, gxy, const0, const1, sharpenOnly);
    imageStore(img_dst, ASU2(gxy), c);
    gxy.x -= 8u;

    CasFilter(c.r, c.g, c.b, gxy, const0, const1, sharpenOnly);
    imageStore(img_dst, ASU2(gxy), c);

#endif
}
