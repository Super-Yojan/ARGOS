using System.Collections.Generic;
using UnityEngine;

namespace Argos.Fleet
{
    /// <summary>
    /// Runtime materials come from Resources assets so the player build keeps the shader.
    /// Shader.Find alone is not enough once a pass fails to compile or a variant is stripped.
    /// </summary>
    public static class ArgosShaders
    {
        static readonly HashSet<string> Logged = new HashSet<string>();

        public static Material Create(string resourceName, params string[] fallbackNames)
        {
            var template = Resources.Load<Material>(resourceName);
            if (Usable(template))
            {
                LogOnce(resourceName, template.shader.name);
                return new Material(template);
            }

            var foundName = template != null && template.shader != null ? template.shader.name : "missing";
            foreach (var name in fallbackNames)
            {
                var shader = Shader.Find(name);
                if (!Usable(shader))
                    continue;
                LogOnce(resourceName, shader.name + " (template was " + foundName + ")");
                return new Material(shader);
            }

            Debug.LogError("ARGOS shader missing: " + resourceName);
            var emergency = Shader.Find("Sprites/Default");
            if (emergency == null)
                emergency = Shader.Find("Unlit/Color");
            if (emergency == null)
                throw new System.InvalidOperationException("ARGOS shader missing: " + resourceName);
            return new Material(emergency);
        }

        static bool Usable(Material material)
        {
            return material != null && Usable(material.shader);
        }

        static bool Usable(Shader shader)
        {
            return shader != null
                && shader.isSupported
                && shader.name != "Hidden/InternalErrorShader";
        }

        static void LogOnce(string resourceName, string shaderName)
        {
            if (!Logged.Add(resourceName))
                return;
            Debug.Log("ARGOS shader " + resourceName + ": " + shaderName);
        }
    }
}
