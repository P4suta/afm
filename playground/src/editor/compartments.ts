import { Compartment, type Extension } from '@codemirror/state';

export interface EngineFeatureSettings {
  readonly structureHighlight: boolean;
  readonly gaijiInlayHints: boolean;
}

export interface EngineFeatures {
  readonly extension: Extension;
  readonly structureHighlight: Extension;
  readonly gaijiInlayHints: Extension;
}

export type EngineFeatureFactory = (
  settings: EngineFeatureSettings,
) => EngineFeatures;

export const engineFeaturesCompartment = new Compartment();
export const structureHighlightCompartment = new Compartment();
export const inlayHintsCompartment = new Compartment();
