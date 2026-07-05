// Promise wrapper over the patched ogv.js VP9 MT decoder module
// (ogv-decoder-video-vp9-mt.js built with outputVideoFrame support).
// Classic-worker script: load with importScripts().
(function(global) {
	'use strict';

	function loadFactory(moduleScript) {
		if (global.OGVDecoderVideoVP9MT) {
			return global.OGVDecoderVideoVP9MT;
		}
		if (typeof importScripts !== 'function') {
			throw new Error('OGVDecoderVideoVP9MT is not loaded and importScripts is unavailable');
		}
		importScripts(moduleScript);
		if (!global.OGVDecoderVideoVP9MT) {
			throw new Error('ogv-decoder-video-vp9-mt.js did not define OGVDecoderVideoVP9MT');
		}
		return global.OGVDecoderVideoVP9MT;
	}

	function drain(module, resolve, reject) {
		return function(ok) {
			var error = module.videoFrameError;
			if (error) {
				module.videoFrameError = null;
				reject(error);
				return;
			}
			// ok=false means "no frame emitted"; a decode error and a legal
			// hidden-frames-only packet are indistinguishable here.
			resolve({ ok: !!ok, frames: module.takeFrameQueue() });
		};
	}

	class OGVVP9VideoFrameDecoder {
		constructor(module) {
			this.module = module;
		}

		static create(options) {
			var moduleScript = new URL(options.moduleScript, global.location.href).href;
			var base = new URL('.', moduleScript).href;
			var factory = loadFactory(moduleScript);
			return factory({
				outputVideoFrame: true,
				videoFormat: options.videoFormat,
				// pthread workers bootstrap by re-importing the main script.
				mainScriptUrlOrBlob: moduleScript,
				locateFile: function(path) {
					return new URL(path, base).href;
				}
			}).then(function(module) {
				return new Promise(function(resolve) {
					module.init(function() {
						resolve(new OGVVP9VideoFrameDecoder(module));
					});
				});
			});
		}

		// Multiple decodes may be in flight: the glue and the wasm dispatch
		// queue submissions and complete them in order. Frames are drained at
		// each completion, so under pipelining a frame can surface on a
		// neighboring packet's result; totals stay exact.
		decode(packet) {
			var module = this.module;
			return new Promise(function(resolve, reject) {
				module.processFrame(packet.data, drain(module, resolve, reject), {
					timestampUs: packet.timestampUs
				});
			});
		}

		// Barrier: completes after every prior decode() has completed.
		flush() {
			var module = this.module;
			return new Promise(function(resolve, reject) {
				module.sync(drain(module, resolve, reject));
			});
		}

		close() {
			if (this.module) {
				this.module.close();
				this.module = null;
			}
		}
	}

	global.OGVVP9VideoFrameDecoder = OGVVP9VideoFrameDecoder;
})(globalThis);
