export function claimsNativePreview({ previewTransport, diagnostics = {} }) {
  const transports = new Set([
    previewTransport,
    ...(Array.isArray(diagnostics.transports) ? diagnostics.transports : []),
  ].filter(Boolean))
  const backings = new Set([
    diagnostics.previewSurfaceBacking,
    ...(Array.isArray(diagnostics.surfaceBackings) ? diagnostics.surfaceBackings : []),
  ].filter(Boolean))

  return transports.has('native-surface') && backings.has('cametal-layer')
}

export function formatTransportHonesty({ previewTransport, diagnostics = {} }) {
  const imagePolls = diagnostics.imagePollDuringSession?.total ?? 0
  if (claimsNativePreview({ previewTransport, diagnostics })) {
    return imagePolls === 0
      ? 'native (0 image polls)'
      : `NOT native (${imagePolls} image polls during native preview claim)`
  }

  const backing = diagnostics.previewSurfaceBacking ?? 'unknown backing'
  const transports = Array.isArray(diagnostics.transports) && diagnostics.transports.length
    ? diagnostics.transports.join(', ')
    : (previewTransport ?? 'unknown transport')
  return `NOT native (${transports}; ${backing}; ${imagePolls} image polls)`
}
