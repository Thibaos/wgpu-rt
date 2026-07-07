@binding(0) @group(0) var output_0 : texture_storage_2d<rgba8unorm, write>;

struct _MatrixStorage_float4x4_ColMajorstd140_0
{
    @align(16) data_0 : array<vec4<f32>, i32(4)>,
};

struct SLANG_ParameterGroup_Camera_std140_0
{
    @align(16) pos_0 : vec3<f32>,
    @align(16) viewInv_0 : _MatrixStorage_float4x4_ColMajorstd140_0,
    @align(16) projInv_0 : _MatrixStorage_float4x4_ColMajorstd140_0,
};

@binding(2) @group(0) var<uniform> Camera_0 : SLANG_ParameterGroup_Camera_std140_0;
struct SLANG_ParameterGroup_TreeParams_std140_0
{
    @align(16) rootNodeIndex_0 : u32,
    @align(4) treeScale_0 : u32,
    @align(16) rootOffset_0 : vec3<i32>,
};

@binding(1) @group(0) var<uniform> TreeParams_0 : SLANG_ParameterGroup_TreeParams_std140_0;
@binding(3) @group(0) var<storage, read> treeNodes_0 : array<u32>;

@binding(4) @group(0) var<storage, read> leafData_0 : array<u32>;

@binding(5) @group(0) var<storage, read> palette_0 : array<vec4<f32>>;

fn firstbithigh_0( value_0 : u32) -> u32
{
    var _S1 : u32 = firstLeadingBit(value_0);
    return _S1;
}

fn GetMirroredPosLocal_0( pos_1 : vec3<f32>,  dir_0 : vec3<f32>,  rangeCheck_0 : bool) -> vec3<f32>
{
    var _S2 : vec3<f32> = (bitcast<vec3<f32>>((((bitcast<vec3<u32>>((pos_1))) ^ (vec3<u32>(u32(8388607)))))));
    var _S3 : bool;
    if(rangeCheck_0)
    {
        _S3 = (any((select(pos_1 >= vec3<f32>(2.0f), vec3<bool>(true), pos_1 < vec3<f32>(1.0f)))));
    }
    else
    {
        _S3 = false;
    }
    var mirrored_0 : vec3<f32>;
    if(_S3)
    {
        mirrored_0 = vec3<f32>(3.0f) - pos_1;
    }
    else
    {
        mirrored_0 = _S2;
    }
    var result_0 : vec3<f32>;
    if((dir_0.x) > 0.0f)
    {
        result_0[i32(0)] = mirrored_0.x;
    }
    else
    {
        result_0[i32(0)] = pos_1.x;
    }
    if((dir_0.y) > 0.0f)
    {
        result_0[i32(1)] = mirrored_0.y;
    }
    else
    {
        result_0[i32(1)] = pos_1.y;
    }
    if((dir_0.z) > 0.0f)
    {
        result_0[i32(2)] = mirrored_0.z;
    }
    else
    {
        result_0[i32(2)] = pos_1.z;
    }
    return result_0;
}

struct Node_0
{
     packedData0_0 : u32,
     packedData1_0 : u32,
     packedData2_0 : u32,
};

fn ReadNode_0( index_0 : u32) -> Node_0
{
    var base_0 : u32 = index_0 * u32(3);
    var n_0 : Node_0;
    n_0.packedData0_0 = treeNodes_0[base_0];
    n_0.packedData1_0 = treeNodes_0[base_0 + u32(1)];
    n_0.packedData2_0 = treeNodes_0[base_0 + u32(2)];
    return n_0;
}

fn GetCellIndex_0( pos_2 : vec3<f32>,  scaleExp_0 : i32) -> i32
{
    var cellPos_0 : vec3<u32> = ((((bitcast<vec3<u32>>((pos_2))) >> (vec3<u32>(u32(scaleExp_0))))) & (vec3<u32>(u32(3))));
    return i32(cellPos_0.x + cellPos_0.y * u32(4) + cellPos_0.z * u32(16));
}

fn Node_PopMask_get_0( this_0 : Node_0) -> u64
{
    return (u64(this_0.packedData1_0) | (((u64(this_0.packedData2_0) << (u32(32))))));
}

fn BitTest_0( value_1 : u64,  shift_0 : u32,  mask_0 : u32) -> bool
{
    var low_0 : u32;
    if(shift_0 < u32(32))
    {
        low_0 = u32(value_1);
    }
    else
    {
        low_0 = u32((value_1 >> (u32(32))));
    }
    return ((((low_0 >> (((shift_0 & (u32(31))))))) & (mask_0))) != u32(0);
}

fn Node_IsLeaf_get_0( this_1 : Node_0) -> bool
{
    return (((this_1.packedData0_0) & (u32(1)))) != u32(0);
}

fn Node_ChildPtr_get_0( this_2 : Node_0) -> u32
{
    return ((this_2.packedData0_0) >> (u32(1)));
}

fn prefix_popcnt64_0( mask_1 : u64,  width_0 : u32) -> u32
{
    var himask_0 : u32 = u32(mask_1);
    var himask_1 : u32;
    var count_0 : u32;
    if(width_0 >= u32(32))
    {
        var _S4 : u32 = countOneBits(himask_0);
        himask_1 = u32((mask_1 >> (u32(32))));
        count_0 = _S4;
    }
    else
    {
        himask_1 = himask_0;
        count_0 = u32(0);
    }
    return count_0 + countOneBits((himask_1 & ((((u32(1) << (((width_0 & (u32(31))))))) - u32(1)))));
}

fn FloorScaleLocal_0( pos_3 : vec3<f32>,  scaleExp_1 : i32) -> vec3<f32>
{
    return (bitcast<vec3<f32>>((((bitcast<vec3<u32>>((pos_3))) & (vec3<u32>(((u32(4294967295) << (u32(scaleExp_1))))))))));
}

fn ReadLeafMaterial_0( node_0 : Node_0,  childIdx_0 : u32) -> u32
{
    var packedIdx_0 : u32 = Node_ChildPtr_get_0(node_0) + prefix_popcnt64_0(Node_PopMask_get_0(node_0), childIdx_0);
    return (((leafData_0[packedIdx_0 / u32(4)] >> ((((packedIdx_0 & (u32(3)))) * u32(8))))) & (u32(255)));
}

struct HitInfo_0
{
     Pos_0 : vec3<f32>,
     Normal_0 : vec3<f32>,
     MaterialId_0 : u32,
};

fn Tree64RayCast_0( worldOrigin_0 : vec3<i32>,  rayPos_0 : vec3<f32>,  rayDir_0 : vec3<f32>) -> HitInfo_0
{
    var _S5 : u64;
    var _S6 : bool;
    var _S7 : vec3<f32> = vec3<f32>((1.0f / f32((i32(1) << ((TreeParams_0.treeScale_0))))));
    var _S8 : vec3<f32> = vec3<f32>(1.0f);
    var origin_0 : vec3<f32> = vec3<f32>(worldOrigin_0) * _S7 + rayPos_0 * _S7 + _S8;
    var _S9 : vec3<f32> = abs(rayDir_0);
    var invDir_0 : vec3<f32> = _S8 / (vec3<f32>(0) - _S9);
    var mirrorMask_0 : u32;
    if((rayDir_0.x) > 0.0f)
    {
        mirrorMask_0 = u32(3);
    }
    else
    {
        mirrorMask_0 = u32(0);
    }
    if((rayDir_0.y) > 0.0f)
    {
        mirrorMask_0 = (mirrorMask_0 | (u32(12)));
    }
    if((rayDir_0.z) > 0.0f)
    {
        mirrorMask_0 = (mirrorMask_0 | (u32(48)));
    }
    var origin_1 : vec3<f32> = GetMirroredPosLocal_0(origin_0, rayDir_0, true);
    var _S10 : vec3<f32> = vec3<f32>(1.0f);
    var _S11 : vec3<f32> = vec3<f32>(1.99999988079071045f);
    var pos_4 : vec3<f32> = clamp(origin_1, _S10, _S11);
    var pos_5 : vec3<f32>;
    var sideDist_0 : vec3<f32>;
    var skipNextHit_0 : bool;
    if((any((pos_4 != origin_1))))
    {
        var t0_0 : vec3<f32> = (vec3<f32>(2.0f) - origin_1) * invDir_0;
        var t1_0 : vec3<f32> = (_S10 - origin_1) * invDir_0;
        var _S12 : f32 = max(max(t0_0.x, t0_0.y), max(t0_0.z, 0.0f));
        var _S13 : vec3<f32> = (vec3<f32>(0) - t0_0);
        var _S14 : bool = _S12 >= (min(min(t1_0.x, t1_0.y), t1_0.z));
        pos_5 = clamp(origin_1 - _S9 * vec3<f32>(_S12), _S10, _S11);
        skipNextHit_0 = _S14;
        sideDist_0 = _S13;
    }
    else
    {
        pos_5 = pos_4;
        skipNextHit_0 = true;
    }
    var stack_0 : array<u32, i32(11)>;
    var childIdx_1 : i32;
    var node_1 : Node_0 = ReadNode_0(TreeParams_0.rootNodeIndex_0);
    var nodeIdx_0 : u32 = TreeParams_0.rootNodeIndex_0;
    var i_0 : i32 = i32(0);
    var scaleExp_2 : i32 = i32(21);
    for(;;)
    {
        if(i_0 < i32(256))
        {
        }
        else
        {
            break;
        }
        var _S15 : i32 = i32(mirrorMask_0);
        var _S16 : i32 = ((GetCellIndex_0(pos_5, scaleExp_2)) ^ (_S15));
        var node_2 : Node_0 = node_1;
        var childIdx_2 : i32 = _S16;
        var nodeIdx_1 : u32 = nodeIdx_0;
        var scaleExp_3 : i32 = scaleExp_2;
        for(;;)
        {
            var _S17 : u64 = Node_PopMask_get_0(node_2);
            _S5 = _S17;
            var _S18 : u32 = u32(childIdx_2);
            var _S19 : bool = BitTest_0(_S17, _S18, u32(1));
            _S6 = _S19;
            var _S20 : bool;
            if(_S19)
            {
                _S20 = !Node_IsLeaf_get_0(node_2);
            }
            else
            {
                _S20 = false;
            }
            if(_S20)
            {
            }
            else
            {
                break;
            }
            stack_0[(scaleExp_3 >> (u32(1)))] = nodeIdx_1;
            var nodeIdx_2 : u32 = Node_ChildPtr_get_0(node_2) + prefix_popcnt64_0(_S17, _S18);
            var scaleExp_4 : i32 = scaleExp_3 - i32(2);
            var _S21 : i32 = ((GetCellIndex_0(pos_5, scaleExp_4)) ^ (_S15));
            node_2 = ReadNode_0(nodeIdx_2);
            childIdx_2 = _S21;
            nodeIdx_1 = nodeIdx_2;
            scaleExp_3 = scaleExp_4;
        }
        var _S22 : bool;
        if(_S6)
        {
            _S22 = Node_IsLeaf_get_0(node_2);
        }
        else
        {
            _S22 = false;
        }
        var _S23 : bool;
        if(_S22)
        {
            _S23 = !skipNextHit_0;
        }
        else
        {
            _S23 = false;
        }
        if(_S23)
        {
            node_1 = node_2;
            childIdx_1 = childIdx_2;
            scaleExp_2 = scaleExp_3;
            break;
        }
        var advScaleExp_0 : i32;
        if(!BitTest_0(_S5, u32((childIdx_2 & (i32(42)))), u32(3342387)))
        {
            advScaleExp_0 = scaleExp_3 + i32(1);
        }
        else
        {
            advScaleExp_0 = scaleExp_3;
        }
        var edgePos_0 : vec3<f32> = FloorScaleLocal_0(pos_5, advScaleExp_0);
        var sideDist_1 : vec3<f32> = (edgePos_0 - origin_1) * invDir_0;
        var _S24 : f32 = sideDist_1.x;
        var _S25 : f32 = sideDist_1.y;
        var _S26 : f32 = sideDist_1.z;
        var _S27 : f32 = min(min(_S24, _S25), _S26);
        var maxSiblBounds_0 : vec3<i32> = (bitcast<vec3<i32>>((edgePos_0)));
        if(_S24 == _S27)
        {
            maxSiblBounds_0[i32(0)] = maxSiblBounds_0[i32(0)] + i32(-1);
        }
        else
        {
            maxSiblBounds_0[i32(0)] = maxSiblBounds_0[i32(0)] + (((i32(1) << (u32(advScaleExp_0)))) - i32(1));
        }
        if(_S25 == _S27)
        {
            maxSiblBounds_0[i32(1)] = maxSiblBounds_0[i32(1)] + i32(-1);
        }
        else
        {
            maxSiblBounds_0[i32(1)] = maxSiblBounds_0[i32(1)] + (((i32(1) << (u32(advScaleExp_0)))) - i32(1));
        }
        if(_S26 == _S27)
        {
            maxSiblBounds_0[i32(2)] = maxSiblBounds_0[i32(2)] + i32(-1);
        }
        else
        {
            maxSiblBounds_0[i32(2)] = maxSiblBounds_0[i32(2)] + (((i32(1) << (u32(advScaleExp_0)))) - i32(1));
        }
        var pos_6 : vec3<f32> = min(origin_1 - _S9 * vec3<f32>(_S27), (bitcast<vec3<f32>>((maxSiblBounds_0))));
        var diffPos_0 : vec3<u32> = ((bitcast<vec3<u32>>((pos_6))) ^ ((bitcast<vec3<u32>>((edgePos_0)))));
        var diffExp_0 : i32 = i32(firstbithigh_0(((((((diffPos_0.x) | ((diffPos_0.y)))) | ((diffPos_0.z)))) & (u32(4289374890)))));
        if(diffExp_0 > scaleExp_3)
        {
            if(diffExp_0 > i32(21))
            {
                node_1 = node_2;
                pos_5 = pos_6;
                childIdx_1 = childIdx_2;
                sideDist_0 = sideDist_1;
                scaleExp_2 = diffExp_0;
                break;
            }
            node_1 = ReadNode_0(stack_0[(diffExp_0 >> (u32(1)))]);
            nodeIdx_0 = stack_0[(diffExp_0 >> (u32(1)))];
            scaleExp_2 = diffExp_0;
        }
        else
        {
            node_1 = node_2;
            nodeIdx_0 = nodeIdx_1;
            scaleExp_2 = scaleExp_3;
        }
        var _S28 : i32 = i_0 + i32(1);
        pos_5 = pos_6;
        skipNextHit_0 = false;
        childIdx_1 = childIdx_2;
        sideDist_0 = sideDist_1;
        i_0 = _S28;
    }
    var hit_0 : HitInfo_0;
    hit_0.MaterialId_0 = u32(0);
    hit_0.Normal_0 = vec3<f32>(0.0f, 0.0f, 0.0f);
    if(Node_IsLeaf_get_0(node_1))
    {
        skipNextHit_0 = scaleExp_2 <= i32(21);
    }
    else
    {
        skipNextHit_0 = false;
    }
    if(skipNextHit_0)
    {
        var pos_7 : vec3<f32> = GetMirroredPosLocal_0(pos_5, rayDir_0, false);
        hit_0.MaterialId_0 = ReadLeafMaterial_0(node_1, u32(childIdx_1));
        hit_0.Pos_0 = pos_7;
        var sideMask_0 : vec3<bool> = vec3<f32>((min(min(sideDist_0.x, sideDist_0.y), sideDist_0.z))) >= sideDist_0;
        hit_0.Normal_0 = vec3<f32>(vec3<i32>(i32(0)));
        if(sideMask_0.x)
        {
            hit_0.Normal_0[i32(0)] = f32(- (vec3<i32>(sign((rayDir_0)))).x);
        }
        if(sideMask_0.y)
        {
            hit_0.Normal_0[i32(1)] = f32(- (vec3<i32>(sign((rayDir_0)))).y);
        }
        if(sideMask_0.z)
        {
            hit_0.Normal_0[i32(2)] = f32(- (vec3<i32>(sign((rayDir_0)))).z);
        }
    }
    return hit_0;
}

@compute
@workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) globalId_0 : vec3<u32>)
{
    var targetSize_0 : vec2<f32>;
    var _S29 : f32 = targetSize_0[i32(0)];
    var _S30 : f32 = targetSize_0[i32(1)];
    {var dim = textureDimensions((output_0));((_S29)) = f32(dim.x);((_S30)) = f32(dim.y);};
    targetSize_0[i32(0)] = _S29;
    targetSize_0[i32(1)] = _S30;
    var _S31 : vec2<u32> = globalId_0.xy;
    var ndc_0 : vec2<f32> = (vec2<f32>(_S31) + vec2<f32>(0.5f)) / targetSize_0 * vec2<f32>(2.0f) - vec2<f32>(1.0f);
    var rayOrigin_0 : vec3<f32> = (((vec4<f32>(0.0f, 0.0f, 0.0f, 1.0f)) * (mat4x4<f32>(Camera_0.viewInv_0.data_0[i32(0)][i32(0)], Camera_0.viewInv_0.data_0[i32(1)][i32(0)], Camera_0.viewInv_0.data_0[i32(2)][i32(0)], Camera_0.viewInv_0.data_0[i32(3)][i32(0)], Camera_0.viewInv_0.data_0[i32(0)][i32(1)], Camera_0.viewInv_0.data_0[i32(1)][i32(1)], Camera_0.viewInv_0.data_0[i32(2)][i32(1)], Camera_0.viewInv_0.data_0[i32(3)][i32(1)], Camera_0.viewInv_0.data_0[i32(0)][i32(2)], Camera_0.viewInv_0.data_0[i32(1)][i32(2)], Camera_0.viewInv_0.data_0[i32(2)][i32(2)], Camera_0.viewInv_0.data_0[i32(3)][i32(2)], Camera_0.viewInv_0.data_0[i32(0)][i32(3)], Camera_0.viewInv_0.data_0[i32(1)][i32(3)], Camera_0.viewInv_0.data_0[i32(2)][i32(3)], Camera_0.viewInv_0.data_0[i32(3)][i32(3)])))).xyz;
    var rayDir_1 : vec3<f32> = normalize((((vec4<f32>(normalize((((vec4<f32>(ndc_0.x, ndc_0.y, 1.0f, 1.0f)) * (mat4x4<f32>(Camera_0.projInv_0.data_0[i32(0)][i32(0)], Camera_0.projInv_0.data_0[i32(1)][i32(0)], Camera_0.projInv_0.data_0[i32(2)][i32(0)], Camera_0.projInv_0.data_0[i32(3)][i32(0)], Camera_0.projInv_0.data_0[i32(0)][i32(1)], Camera_0.projInv_0.data_0[i32(1)][i32(1)], Camera_0.projInv_0.data_0[i32(2)][i32(1)], Camera_0.projInv_0.data_0[i32(3)][i32(1)], Camera_0.projInv_0.data_0[i32(0)][i32(2)], Camera_0.projInv_0.data_0[i32(1)][i32(2)], Camera_0.projInv_0.data_0[i32(2)][i32(2)], Camera_0.projInv_0.data_0[i32(3)][i32(2)], Camera_0.projInv_0.data_0[i32(0)][i32(3)], Camera_0.projInv_0.data_0[i32(1)][i32(3)], Camera_0.projInv_0.data_0[i32(2)][i32(3)], Camera_0.projInv_0.data_0[i32(3)][i32(3)])))).xyz), 0.0f)) * (mat4x4<f32>(Camera_0.viewInv_0.data_0[i32(0)][i32(0)], Camera_0.viewInv_0.data_0[i32(1)][i32(0)], Camera_0.viewInv_0.data_0[i32(2)][i32(0)], Camera_0.viewInv_0.data_0[i32(3)][i32(0)], Camera_0.viewInv_0.data_0[i32(0)][i32(1)], Camera_0.viewInv_0.data_0[i32(1)][i32(1)], Camera_0.viewInv_0.data_0[i32(2)][i32(1)], Camera_0.viewInv_0.data_0[i32(3)][i32(1)], Camera_0.viewInv_0.data_0[i32(0)][i32(2)], Camera_0.viewInv_0.data_0[i32(1)][i32(2)], Camera_0.viewInv_0.data_0[i32(2)][i32(2)], Camera_0.viewInv_0.data_0[i32(3)][i32(2)], Camera_0.viewInv_0.data_0[i32(0)][i32(3)], Camera_0.viewInv_0.data_0[i32(1)][i32(3)], Camera_0.viewInv_0.data_0[i32(2)][i32(3)], Camera_0.viewInv_0.data_0[i32(3)][i32(3)])))).xyz);
    var rayOrigin_1 : vec3<f32> = vec3<f32>(rayOrigin_0.x, rayOrigin_0.z, rayOrigin_0.y);
    var _S32 : f32 = rayDir_1.x;
    var _S33 : f32 = rayDir_1.z;
    var _S34 : f32 = rayDir_1.y;
    var hit_1 : HitInfo_0 = Tree64RayCast_0(vec3<i32>(floor(rayOrigin_1)) - TreeParams_0.rootOffset_0, fract(rayOrigin_1), vec3<f32>(_S32, _S33, _S34));
    var _S35 : vec4<f32> = vec4<f32>(_S32, _S33, _S34, 1.0f);
    var color_0 : vec4<f32>;
    if((any(((hit_1.Normal_0) != vec3<f32>(0.0f, 0.0f, 0.0f)))))
    {
        color_0 = vec4<f32>(palette_0[hit_1.MaterialId_0].xyz * vec3<f32>(max(dot(vec3<f32>(hit_1.Normal_0.x, hit_1.Normal_0.z, hit_1.Normal_0.y), normalize(vec3<f32>(0.5f, 1.0f, 0.30000001192092896f))), 0.10000000149011612f)), 1.0f);
    }
    else
    {
        color_0 = _S35;
    }
    textureStore((output_0), (_S31), (color_0));
    return;
}

