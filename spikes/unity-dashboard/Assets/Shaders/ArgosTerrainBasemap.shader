Shader "Argos/TerrainBasemap"
{
    Properties
    {
        _MainTex ("Basemap", 2D) = "gray" {}
        _Tint ("Tint", Color) = (0.45, 0.48, 0.52, 1)
        _Saturation ("Saturation", Range(0, 1)) = 0.28
    }
    SubShader
    {
        Tags { "Queue"="Geometry" "RenderType"="Opaque" }
        LOD 100
        ZWrite On
        Cull Off

        Pass
        {
            CGPROGRAM
            #pragma vertex vert
            #pragma fragment frag
            #include "UnityCG.cginc"

            sampler2D _MainTex;
            fixed4 _Tint;
            float _Saturation;

            struct appdata
            {
                float4 vertex : POSITION;
                float2 uv : TEXCOORD0;
            };

            struct v2f
            {
                float4 pos : SV_POSITION;
                float2 uv : TEXCOORD0;
            };

            v2f vert(appdata input)
            {
                v2f output;
                output.pos = UnityObjectToClipPos(input.vertex);
                output.uv = input.uv;
                return output;
            }

            fixed4 frag(v2f input) : SV_Target
            {
                fixed4 sample = tex2D(_MainTex, input.uv);
                float luma = dot(sample.rgb, float3(0.30, 0.52, 0.18));
                float3 muted = lerp(float3(luma, luma, luma), sample.rgb, _Saturation);
                return fixed4(muted * _Tint.rgb, 1);
            }
            ENDCG
        }
    }
}
