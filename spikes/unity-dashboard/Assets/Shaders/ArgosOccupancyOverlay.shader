Shader "Argos/OccupancyOverlay"
{
    Properties
    {
        _MainTex ("Grid", 2D) = "black" {}
        _Color ("Tint", Color) = (1, 1, 1, 1)
        _Border ("Border", Color) = (0.75, 0.82, 0.86, 0.72)
    }
    SubShader
    {
        Tags { "Queue"="Transparent" "RenderType"="Transparent" }
        LOD 100
        ZWrite Off
        Blend SrcAlpha OneMinusSrcAlpha
        Cull Off
        Offset -1, -1

        Pass
        {
            CGPROGRAM
            #pragma vertex vert
            #pragma fragment frag
            #include "UnityCG.cginc"

            sampler2D _MainTex;
            fixed4 _Color;
            fixed4 _Border;

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
                float edge = min(min(input.uv.x, 1.0 - input.uv.x), min(input.uv.y, 1.0 - input.uv.y));
                if (edge < 0.016)
                {
                    fixed4 line = _Border;
                    line.a *= _Color.a;
                    return line;
                }

                fixed4 sample = tex2D(_MainTex, input.uv);
                sample.rgb *= _Color.rgb;
                sample.a *= _Color.a;
                return sample;
            }
            ENDCG
        }
    }
}
