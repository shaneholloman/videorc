import { ThemeProvider, useTheme } from 'next-themes'
import { useEffect, type ReactElement } from 'react'

import { AppShell } from '@/components/app-shell'
import { GlassWallpaperUnderlay } from '@/components/glass-wallpaper'
import { Toaster } from '@/components/ui/sonner'
import { TooltipProvider } from '@/components/ui/tooltip'
import { BackgroundAssetsProvider } from '@/hooks/use-background-assets'
import { StudioProvider } from '@/hooks/use-studio'
import { STORAGE_KEYS } from '@/lib/capture'

// The OS vibrancy material tints by nativeTheme, not by our CSS class; keep
// it in step with the app theme so the glass blur always matches.
function NativeThemeSync(): null {
  const { resolvedTheme } = useTheme()
  useEffect(() => {
    if (resolvedTheme === 'dark' || resolvedTheme === 'light') {
      void window.videorc?.setNativeTheme?.(resolvedTheme)
    }
  }, [resolvedTheme])
  return null
}

export function App(): ReactElement {
  return (
    <ThemeProvider
      attribute="class"
      // Dark glass is the design's default expression; light stays one toggle
      // away as its structural twin (videorc-design skill).
      defaultTheme="dark"
      enableSystem
      storageKey={STORAGE_KEYS.theme}
    >
      <NativeThemeSync />
      <GlassWallpaperUnderlay />
      <TooltipProvider>
        <BackgroundAssetsProvider>
          <StudioProvider>
            <AppShell />
            <Toaster richColors position="bottom-right" />
          </StudioProvider>
        </BackgroundAssetsProvider>
      </TooltipProvider>
    </ThemeProvider>
  )
}
