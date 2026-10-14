using System.IO;
using Argos.Fleet;
using UnityEditor;
using UnityEditor.Build;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.Rendering;

namespace Argos.Fleet.EditorTools
{
    public static class FleetSceneSetup
    {
        const string ScenePath = "Assets/Scenes/FleetOverview.unity";

        public static void CreateFleetScene()
        {
            BuildSceneAsset();
            EditorApplication.Exit(0);
        }

        public static void PrepareMacPlayer()
        {
            BuildSceneAsset();
            PlayerSettings.companyName = "ARGOS";
            PlayerSettings.productName = "ArgosFleetDashboard";
            PlayerSettings.fullScreenMode = FullScreenMode.Windowed;
            PlayerSettings.defaultScreenWidth = 1280;
            PlayerSettings.defaultScreenHeight = 720;
            PlayerSettings.runInBackground = true;
            PlayerSettings.SetScriptingBackend(NamedBuildTarget.Standalone, ScriptingImplementation.Mono2x);
            AssetDatabase.SaveAssets();
            EditorApplication.Exit(0);
        }

        public static void PrepareLinuxPlayer()
        {
            BuildSceneAsset();
            PlayerSettings.companyName = "ARGOS";
            PlayerSettings.productName = "ArgosFleetDashboard";
            PlayerSettings.fullScreenMode = FullScreenMode.Windowed;
            PlayerSettings.defaultScreenWidth = 1280;
            PlayerSettings.defaultScreenHeight = 720;
            PlayerSettings.runInBackground = true;
            // ProjectSettings defaults to Mono so a Mac without the IL2CPP module can build.
            // The Linux player path opts into IL2CPP here.
            PlayerSettings.SetScriptingBackend(NamedBuildTarget.Standalone, ScriptingImplementation.IL2CPP);
            PlayerSettings.SetUseDefaultGraphicsAPIs(BuildTarget.StandaloneLinux64, false);
            PlayerSettings.SetGraphicsAPIs(BuildTarget.StandaloneLinux64, new[] { GraphicsDeviceType.OpenGLCore });
            AssetDatabase.SaveAssets();
            EditorApplication.Exit(0);
        }

        public static void BuildSceneAsset()
        {
            Directory.CreateDirectory("Assets/Scenes");
            var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);

            var cameraObject = new GameObject("Main Camera");
            cameraObject.tag = "MainCamera";
            var camera = cameraObject.AddComponent<Camera>();
            cameraObject.AddComponent<AudioListener>();
            camera.clearFlags = CameraClearFlags.SolidColor;
            camera.backgroundColor = new Color(0.015f, 0.02f, 0.035f, 1f);
            camera.fieldOfView = 40f;
            camera.nearClipPlane = 0.1f;
            camera.farClipPlane = 250f;
            // High enough that all four rows of the 20-rover grid sit inside a 1280x720 frame.
            cameraObject.transform.position = new Vector3(0f, 36f, -12f);
            cameraObject.transform.rotation = Quaternion.Euler(72f, 0f, 0f);

            var lightObject = new GameObject("Directional Light");
            var light = lightObject.AddComponent<Light>();
            light.type = LightType.Directional;
            light.intensity = 1.1f;
            light.color = new Color(0.78f, 0.9f, 1f);
            lightObject.transform.rotation = Quaternion.Euler(48f, -28f, 0f);

            var fleet = new GameObject("FleetDashboard");
            fleet.AddComponent<FleetDashboard>();
            ReferenceRuntimeShaders();

            EditorSceneManager.SaveScene(scene, ScenePath);
            EditorBuildSettings.scenes = new[]
            {
                new EditorBuildSettingsScene(ScenePath, true),
            };
            AssetDatabase.SaveAssets();
        }

        // Disabled renderers still pull these materials into the player, so Shader.Find
        // and Resources.Load cannot lose them to variant stripping.
        static void ReferenceRuntimeShaders()
        {
            var names = new[]
            {
                "ArgosFleetUnlit",
                "ArgosFleetOpaque",
                "ArgosTerrainBasemap",
                "ArgosOccupancyOverlay",
                "ArgosCampusBuilding",
            };
            foreach (var name in names)
            {
                var path = "Assets/Resources/" + name + ".mat";
                var material = AssetDatabase.LoadAssetAtPath<Material>(path);
                if (material == null)
                {
                    Debug.LogError("ARGOS shader material missing: " + path);
                    continue;
                }

                var marker = GameObject.CreatePrimitive(PrimitiveType.Quad);
                marker.name = "shader-ref-" + name;
                marker.SetActive(false);
                marker.GetComponent<Renderer>().sharedMaterial = material;
                var collider = marker.GetComponent<Collider>();
                if (collider != null)
                    Object.DestroyImmediate(collider);
            }
        }
    }
}
