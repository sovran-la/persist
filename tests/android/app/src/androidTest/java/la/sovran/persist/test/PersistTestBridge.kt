package la.sovran.persist.test

/**
 * JNI bridge to the persist native test library.
 */
object PersistTestBridge {
    init {
        System.loadLibrary("persist_android_test")
    }

    /** Initialize persist's Android layer with an app Context. */
    external fun nativeInit(context: android.content.Context)

    /** Run all SharedPreferencesStore tests. Returns "" on success, error on failure. */
    external fun nativeRunTests(): String
}
