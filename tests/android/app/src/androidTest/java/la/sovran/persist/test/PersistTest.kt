package la.sovran.persist.test

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.BeforeClass
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PersistTest {

    companion object {
        @JvmStatic
        @BeforeClass
        fun initNative() {
            val ctx = InstrumentationRegistry.getInstrumentation().targetContext
            PersistTestBridge.nativeInit(ctx)
        }
    }

    @Test
    fun allTypesRoundTrip() {
        val result = PersistTestBridge.nativeRunTests()
        assertEquals("Native test failure: $result", "", result)
    }
}
