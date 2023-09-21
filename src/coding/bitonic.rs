//! This module implements an arithmetic code such as the one in zpaq, and
//! described by Matt Mahoney: https://mattmahoney.net/dc/dce.html#Section_32

pub struct BitonicEncoder<'a> {
    /// The output bitstream.
    output: &'a mut Vec<u8>,
    /// The low side of the range.
    low: u32,
    /// The high side of the range.
    high: u32,
}

impl<'a> BitonicEncoder<'a> {
    pub fn new(output: &'a mut Vec<u8>) -> Self {
        Self {
            output,
            low: 0,
            high: 0xffffffff,
        }
    }

    /// Encode the bit 'bit' with probability 'prob' in the range 0..65536.
    pub fn encode(&mut self, bit: bool, prob: u16) {
        assert!(self.high > self.low);

        let gap = (self.high - self.low) as u64;
        let scale = (gap * prob as u64) >> 16;
        let mid = self.low + scale as u32;
        assert!(self.high > mid && mid >= self.low);

        // Pick the half:
        if bit {
            self.high = mid;
        } else {
            self.low = mid + 1;
        }

        // Write the identical leading bytes.
        while (self.high ^ self.low) < (1 << 24) {
            self.output.push((self.high >> 24) as u8);
            self.high = (self.high << 8) + 0xff;
            self.low <<= 8;
        }
    }

    pub fn finalize(&mut self) {
        self.encode(true, 0);
    }

    pub fn encode_array(&mut self, vals: &[bool], prob: &[u16]) {
        assert_eq!(vals.len(), prob.len());
        for i in 0..vals.len() {
            self.encode(vals[i], prob[i]);
        }
        self.finalize();
    }
}

pub struct BitonicDecoder<'a> {
    /// The input bit stream (read from the beginning).
    input: &'a [u8],
    /// Marks the location in the bitstream.
    cursor: usize,
    /// The low point of the range.
    low: u32,
    /// The high point of the range.
    high: u32,
    /// The current state.
    state: u32,
}

impl<'a> BitonicDecoder<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        assert!(input.len() >= 4);
        let mut cursor = 0;
        let mut state: u32 = 0;
        for _ in 0..4 {
            state = state << 8 | input[cursor] as u32;
            cursor += 1;
        }

        Self {
            input,
            cursor,
            low: 0,
            high: 0xffffffff,
            state,
        }
    }

    /// Decode one bit with a probability 'prob' in the range 0..65536.
    pub fn decode(&mut self, prob: u16) -> Option<bool> {
        assert!(self.high > self.low);
        assert!(self.high >= self.state && self.low <= self.state);
        let gap = (self.high - self.low) as u64;
        let scale = (gap * prob as u64) >> 16;
        let mid = self.low + scale as u32;
        assert!(self.high > mid && mid >= self.low);

        let bit = self.state <= mid;
        // Pick the half:
        if bit {
            self.high = mid;
        } else {
            self.low = mid + 1;
        }

        // Clear the identical leading bytes.
        while (self.high ^ self.low) < (1 << 24) {
            // Not enough bits in the input.
            if self.cursor == self.input.len() {
                return None;
            }
            self.high = (self.high << 8) + 0xff;
            self.low <<= 8;
            self.state = (self.state << 8) + self.input[self.cursor] as u32;
            self.cursor += 1;
        }

        Some(bit)
    }

    pub fn decode_array(&mut self, prob: &[u16]) -> Option<Vec<bool>> {
        let mut res = Vec::new();
        for &p in prob {
            res.push(self.decode(p)?);
        }
        Some(res)
    }
}

#[test]
fn test_encoder_decoder() {
    let mut stream = Vec::new();
    let mut encoder = BitonicEncoder::new(&mut stream);
    encoder.encode(false, 255);
    encoder.encode(false, 255);
    encoder.encode(true, 255);
    encoder.encode(false, 255);
    encoder.finalize();

    let mut decoder = BitonicDecoder::new(&stream);
    let _ = decoder.decode(255);
    let _ = decoder.decode(255);
    let _ = decoder.decode(255);
    let _ = decoder.decode(255);
}

#[test]
fn test_encoder_decoder_array() {
    // Define a few probabilities (in the range 0..1<<16)
    let p = 60000_u16;
    let q = 1400_u16;
    let r = 25120_u16;

    // A test vector.
    let test_vector = [
        true, false, true, false, true, true, false, true, false, true, false,
        true, false, false, false, true, false, false,
    ];

    {
        let mut stream = Vec::new();

        let mut encoder = BitonicEncoder::new(&mut stream);
        let prob = [p, p, p, p, p, p, p, p, p, p, p, p, p, p, p, p, p, p];

        encoder.encode_array(&test_vector, &prob);

        let mut decoder = BitonicDecoder::new(&stream);
        let res = decoder.decode_array(&prob).unwrap();
        assert_eq!(res, test_vector);
    }

    {
        let mut stream = Vec::new();
        let mut encoder = BitonicEncoder::new(&mut stream);
        let prob = [p, q, q, r, p, p, r, r, p, p, q, q, q, q, r, p, q, p];

        encoder.encode_array(&test_vector, &prob);

        let mut decoder = BitonicDecoder::new(&stream);
        let res = decoder.decode_array(&prob).unwrap();
        assert_eq!(res, test_vector);
    }
}

/// A simple predictor for the probability of the next bit.
pub struct Model<const CONTEXT_SIZE_BITS: usize, const LIMIT: usize> {
    cache: Vec<(u16, u16)>,
}

impl<const CTX_SIZE_BITS: usize, const LIMIT: usize>
    Model<CTX_SIZE_BITS, LIMIT>
{
    pub fn new() -> Self {
        Self {
            cache: vec![(1, 1); 1 << CTX_SIZE_BITS],
        }
    }

    /// Return a probability prediction in the 16-bit range using the
    /// 'CTX_SIZE_BITS' LSB bits in 'ctx'.
    pub fn predict(&self, ctx: u64) -> u16 {
        let key = ctx % (1 << CTX_SIZE_BITS);
        let (set, cnt) = self.cache[key as usize];
        ((set as u64 * (1 << 16)) / (1 + cnt as u64)) as u16
    }

    /// Update the probability of the context 'ctx', considering the first
    /// 'CTX_SIZE_BITS' LSB bits, with the bit 'bit'.
    pub fn update(&mut self, ctx: u64, bit: u16) {
        let key = ctx % (1 << CTX_SIZE_BITS);
        let (set, cnt) = &mut self.cache[key as usize];
        *cnt += 1;
        *set += bit & 1;
        if *cnt as usize >= LIMIT {
            // Add one to prevent division by zero.
            *set /= 2;
            *cnt /= 2;
        }
    }
}

#[test]
fn test_simple_model() {
    {
        let mut model = Model::<7, 1024>::new();
        for _ in 0..10000 {
            model.update(0, 1);
            model.update(0, 0);
        }
        // The prediction needs to be close to the mid point.
        let pred = model.predict(0);
        assert!(pred > 32_300 && pred < 32_999);
    }

    {
        let mut model = Model::<7, 256>::new();
        for _ in 0..10000 {
            model.update(0, 0);
        }
        // The prediction needs to be close to zero.
        let pred = model.predict(0);
        assert_eq!(pred, 0);
    }

    {
        let mut model = Model::<7, 256>::new();
        for _ in 0..10000 {
            model.update(0, 1);
        }
        // The prediction needs to be close to one.
        let pred = model.predict(0);
        assert!(pred > 65_000);
    }
}
