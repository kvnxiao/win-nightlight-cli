$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class ActiveDisplayRates {
    [DllImport("user32.dll")]
    static extern int GetDisplayConfigBufferSizes(uint flags, out uint paths, out uint modes);
    [DllImport("user32.dll")]
    static extern int QueryDisplayConfig(uint flags, ref uint paths, IntPtr pathData, ref uint modes, IntPtr modeData, IntPtr topology);
    const uint QDC_ONLY_ACTIVE_PATHS = 2;
    // DISPLAYCONFIG_PATH_INFO is 72 bytes: target mode index at 32, refresh rational at 48.
    // DISPLAYCONFIG_MODE_INFO is 64 bytes: target vSyncFreq rational at 32.
    public static List<string> Read() {
        uint paths, modes;
        int error = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, out paths, out modes);
        if (error != 0) throw new Exception("GetDisplayConfigBufferSizes: " + error);
        IntPtr pathData = Marshal.AllocHGlobal(checked((int)paths * 72));
        IntPtr modeData = Marshal.AllocHGlobal(checked((int)modes * 64));
        var rates = new List<string>();
        try {
            error = QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, ref paths, pathData, ref modes, modeData, IntPtr.Zero);
            if (error != 0) throw new Exception("QueryDisplayConfig: " + error);
            for (int i = 0; i < paths; i++) {
                IntPtr path = IntPtr.Add(pathData, i * 72);
                int index = Marshal.ReadInt32(path, 32);
                string rate = Marshal.ReadInt32(path, 48) + "/" + Marshal.ReadInt32(path, 52);
                if (index >= 0 && index < modes) {
                    IntPtr mode = IntPtr.Add(modeData, index * 64);
                    if (Marshal.ReadInt32(mode) == 2) {
                        rate += " path, " + Marshal.ReadInt32(mode, 32) + "/" + Marshal.ReadInt32(mode, 36) + " signal";
                    }
                }
                rates.Add(rate);
            }
        } finally {
            Marshal.FreeHGlobal(modeData);
            Marshal.FreeHGlobal(pathData);
        }
        return rates;
    }
}
"@
$os = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
[ordered]@{
    windows_display_version = $os.DisplayVersion
    windows_build = "$($os.CurrentBuild).$($os.UBR)"
    gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object {
        [ordered]@{
            name = $_.Name
            driver_version = $_.DriverVersion
            width = $_.CurrentHorizontalResolution
            height = $_.CurrentVerticalResolution
            refresh_hz = $_.CurrentRefreshRate
        }
    })
    active_display_refresh_rates_hz = @([ActiveDisplayRates]::Read())
} | ConvertTo-Json -Depth 4 -Compress
