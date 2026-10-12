using System;
using System.Runtime.InteropServices;
using System.Text;
using UnityEngine;

namespace Argos.Fleet
{
    /// <summary>
    /// Polls libargos_unity_bridge. The native side never calls back into C#,
    /// so the same entry points are safe under Mono and IL2CPP.
    /// </summary>
    public static class ZenohBridge
    {
        const int BufferBytes = 262144;

        static readonly byte[] Buffer = new byte[BufferBytes];
        static bool _open;

        public static bool IsOpen => _open;

        public static bool Open(string endpoint, string prefix)
        {
            endpoint = string.IsNullOrEmpty(endpoint) ? "tcp/127.0.0.1:7447" : endpoint;
            prefix = string.IsNullOrEmpty(prefix) ? "terra/rover" : prefix;
            try
            {
                var code = argos_bridge_open(endpoint, prefix);
                _open = code == 0;
                if (!_open)
                    Debug.LogError("ARGOS zenoh open failed (" + code + "): " + LastError());
                return _open;
            }
            catch (DllNotFoundException exception)
            {
                _open = false;
                Debug.LogError("ARGOS zenoh plugin missing: " + exception.Message);
                return false;
            }
            catch (EntryPointNotFoundException exception)
            {
                _open = false;
                Debug.LogError("ARGOS zenoh plugin is missing an entry point: " + exception.Message);
                return false;
            }
        }

        public static void Close()
        {
            if (!_open)
                return;
            try
            {
                argos_bridge_close();
            }
            catch (DllNotFoundException)
            {
            }
            _open = false;
        }

        public static bool Connected
        {
            get
            {
                if (!_open)
                    return false;
                try
                {
                    return argos_bridge_connected() != 0;
                }
                catch (DllNotFoundException)
                {
                    return false;
                }
            }
        }

        public static string Link()
        {
            if (!_open)
                return "disconnected";
            try
            {
                var scratch = new byte[64];
                var length = argos_bridge_link(scratch, (UIntPtr)scratch.Length);
                if (length < 0)
                    return "disconnected";
                return Encoding.UTF8.GetString(scratch, 0, length);
            }
            catch (DllNotFoundException)
            {
                return "disconnected";
            }
        }

        /// <summary>
        /// Latest fleet JSON, or null when the session is closed or the plugin is absent.
        /// An open session with no poses yet returns a JSON object whose rover list is empty.
        /// </summary>
        public static string PollJson()
        {
            if (!_open)
                return null;
            try
            {
                var length = argos_bridge_poll(Buffer, (UIntPtr)Buffer.Length);
                if (length == -1)
                    return null;
                if (length < 0)
                {
                    Debug.LogError("ARGOS zenoh snapshot needs " + (-length) + " bytes.");
                    return null;
                }
                return Encoding.UTF8.GetString(Buffer, 0, length);
            }
            catch (DllNotFoundException exception)
            {
                Debug.LogError("ARGOS zenoh plugin missing: " + exception.Message);
                _open = false;
                return null;
            }
        }

        public static string LastError()
        {
            try
            {
                var scratch = new byte[512];
                var length = argos_bridge_last_error(scratch, (UIntPtr)scratch.Length);
                if (length <= 0)
                    return string.Empty;
                return Encoding.UTF8.GetString(scratch, 0, length);
            }
            catch (DllNotFoundException)
            {
                return "native plugin not loaded";
            }
        }

        [DllImport("argos_unity_bridge", CallingConvention = CallingConvention.Cdecl)]
        static extern int argos_bridge_open(
            [MarshalAs(UnmanagedType.LPStr)] string endpoint,
            [MarshalAs(UnmanagedType.LPStr)] string prefix);

        [DllImport("argos_unity_bridge", CallingConvention = CallingConvention.Cdecl)]
        static extern void argos_bridge_close();

        [DllImport("argos_unity_bridge", CallingConvention = CallingConvention.Cdecl)]
        static extern int argos_bridge_connected();

        [DllImport("argos_unity_bridge", CallingConvention = CallingConvention.Cdecl)]
        static extern int argos_bridge_link(byte[] buffer, UIntPtr capacity);

        [DllImport("argos_unity_bridge", CallingConvention = CallingConvention.Cdecl)]
        static extern int argos_bridge_poll(byte[] buffer, UIntPtr capacity);

        [DllImport("argos_unity_bridge", CallingConvention = CallingConvention.Cdecl)]
        static extern int argos_bridge_last_error(byte[] buffer, UIntPtr capacity);
    }
}
